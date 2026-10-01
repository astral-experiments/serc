use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

#[path = "common/optimization.rs"]
mod cpython;

use cpython::{
    LEVELS, VERSIONS, assert_matches_cpython, assert_matches_cpython_versions,
    assert_outputs_match, assert_runs, compile_both, oracle, probe, run_successfully,
};
use serc::{
    CompileOptions, OptimizationLevel, PythonVersion, compile, compile_bytes_with_options,
    compile_bytes_with_path_and_options, compile_with_options,
};

#[test]
fn split_line_nested_captures() {
    for scope in ["def", "async def"] {
        for guard in ["", " if ready"] {
            let source = format!(
                "{scope} sample(subject, ready):\n    match subject:\n        case {{\n            \"kind\": \"entry\",\n            \"body\": {{\"shape\": None, \"items\": [{{\"kind\": \"node\", \"value\": int(value)}}]}} as data,\n        }}{guard}:\n            return data, value\n    return None\n"
            );
            assert_matches_cpython(&source, "split_line_capture.py", &LEVELS);
        }
    }
}

#[test]
fn nested_mapping_capture_rotations() {
    for scope in ["def", "async def"] {
        for (pattern, values) in [
            (
                "{\"left\": int(left), \"right\": int(right)} as whole",
                "whole, left, right",
            ),
            (
                "{\"payload\": {\"items\": [*parts]} as payload}",
                "payload, parts",
            ),
            (
                "{\"payload\": {\"head\": head, \"items\": [*parts]} as payload}",
                "head, payload, parts",
            ),
        ] {
            for multiline in [false, true] {
                let pattern = if multiline {
                    pattern.replace(", ", ",\n            ")
                } else {
                    pattern.to_string()
                };
                for guard in ["", " if ready"] {
                    let source = format!(
                        "{scope} sample(subject, ready):\n    match subject:\n        case {pattern}{guard}:\n            return {values}\n    return None\n"
                    );
                    assert_matches_cpython(&source, "nested_mapping_capture.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn managed_first_arm_preserves_unlocated_else_call_footer() {
    for scope in ["def", "async def"] {
        for outer in ["flag", "value is not None"] {
            for inner in ["ready", "value == state.first"] {
                for first in [
                    "        with manager(value):\n            operation(value)\n",
                    "        with manager(value) as result:\n            operation(result)\n",
                    "        with manager(value):\n            operation(value)\n        if ready:\n            assert value\n        else:\n            assert state.first\n        with manager(value):\n            finish(value)\n",
                ] {
                    for tail in ["", "    assert value\n"] {
                        let source = format!(
                            "{scope} sample(flag, ready, value, state):\n    if {outer}:\n{first}    else:\n        if {inner}:\n            assert state.ready\n        finish(value)\n{tail}"
                        );
                        assert_matches_cpython_versions(
                            &source,
                            "managed_first_arm_else_call.py",
                            &LEVELS,
                            &[PythonVersion::Py312],
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn loop_guard_prefix_preserves_original_scope_footer() {
    for scope in ["def", "async def"] {
        for guard in ["other", "value == 2"] {
            for prefix in [
                "assert value",
                "value = other",
                "work(value)",
                "value = work(value)",
                "value = await work(value)",
            ] {
                if scope == "def" && prefix.contains("await") {
                    continue;
                }
                let source = format!(
                    "{scope} sample(flag, other, value, state, items):\n    if flag:\n        assert value\n    if other:\n        for item, part in items:\n            {prefix}\n            assert value\n            if {guard}:\n                assert state.ready\n"
                );
                assert_matches_cpython_versions(
                    &source,
                    "loop_guard_prefix_footer.py",
                    &LEVELS,
                    &[PythonVersion::Py312],
                );
            }
        }
    }
}

#[test]
fn erased_loop_guard_preserves_original_scope_footer() {
    for scope in ["def", "async def"] {
        for first in ["flag", "value == 1"] {
            for guard in ["other", "value == 2"] {
                for (target, iterator) in [("item", "items"), ("item, part", "state.items")] {
                    let source = format!(
                        "{scope} sample(flag, other, value, state, items):\n    if {first}:\n        assert value\n    if other:\n        for {target} in {iterator}:\n            if {guard}:\n                assert value\n"
                    );
                    assert_matches_cpython_versions(
                        &source,
                        "erased_loop_guard_footer.py",
                        &LEVELS,
                        &[PythonVersion::Py312],
                    );
                }
            }
        }
    }
}

#[test]
fn terminal_elif_guard_preserves_unlocated_call_footer() {
    for scope in ["def", "async def"] {
        for first in ["flag", "value in {1, 2}"] {
            for inner in ["other", "value in {3, 4}"] {
                for last in ["ready", "value == 5"] {
                    let source = format!(
                        "{scope} sample(flag, other, ready, value, state):\n    if {first}:\n        state.finish(value)\n    elif other:\n        if {inner}:\n            assert value\n        if {last}:\n            state.finish(value)\n"
                    );
                    assert_matches_cpython_versions(
                        &source,
                        "terminal_elif_call_footer.py",
                        &LEVELS,
                        &[PythonVersion::Py312],
                    );
                }
            }
        }
    }
}

#[test]
fn surviving_while_break_enters_nonempty_finally() {
    for body in [
        "while True:\n            if state.done or state.error:\n                break\n            pass",
        "while state.active and ready(items):\n            if state.done or state.error:\n                break\n            pass",
        "while True:\n            with manager:\n                if state.done:\n                    break\n            operation(items)",
        "while state.active and ready(items):\n            if state.outer:\n                if state.done or state.error:\n                    break\n            operation(items)",
    ] {
        for cleanup in [
            "cleanup(items)",
            "try:\n            cleanup(items)\n        except Other:\n            recover(items)",
            "with manager:\n            cleanup(items)",
        ] {
            let source = format!(
                "def sample(items, state, manager):\n    try:\n        {body}\n    except Error as problem:\n        pass\n    finally:\n        {cleanup}\n"
            );
            assert_matches_cpython(&source, "surviving_break_cleanup.py", &LEVELS);
        }
    }
}

#[test]
fn erased_first_arm_preserves_unlocated_else_call_footer() {
    for scope in ["def", "async def"] {
        for outer in ["flag", "value == state.first"] {
            for inner in ["ready", "value == state.second"] {
                for tail in ["", "        assert value\n"] {
                    let source = format!(
                        "{scope} sample(flag, ready, value, state):\n    if {outer}:\n        assert value\n    else:\n        if {inner}:\n            assert state.ready\n        finish(value)\n{tail}"
                    );
                    assert_matches_cpython_versions(
                        &source,
                        "erased_first_arm_else_call.py",
                        &LEVELS,
                        &[PythonVersion::Py312],
                    );
                }
            }
        }
    }
}

#[test]
fn erased_try_finally_arm_preserves_original_scope_footer() {
    for scope in ["def", "async def"] {
        for condition in [
            "flag and (other or ready)",
            "value == 1 and (other == 2 or ready)",
        ] {
            for cleanup in ["cleanup(value)", "state.cache.clear()"] {
                for prefix in [
                    "",
                    "        prepare({item: item.value for item in items})\n",
                ] {
                    let work = if scope == "async def" {
                        "await operation(value)"
                    } else {
                        "operation(value)"
                    };
                    for body in [
                        String::new(),
                        format!(
                            "            {work}\n            value = next((item for item in items if item.ready))\n"
                        ),
                    ] {
                        let source = format!(
                            "{scope} sample(flag, other, ready, value, state, items):\n    if flag:\n        if other:\n            assert value\n    elif ready:\n        assert value\n    if {condition}:\n{prefix}        try:\n{body}            assert value is not None and value.ready\n        finally:\n            {cleanup}\n"
                        );
                        assert_matches_cpython_versions(
                            &source,
                            "erased_try_finally_footer.py",
                            &LEVELS,
                            &[PythonVersion::Py312],
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn nested_erased_elif_arms_preserve_original_scope_footer() {
    for scope in ["def", "async def"] {
        for outer in ["ready", "value == [state.first, state.second]"] {
            for inner in ["other", "value == state.second"] {
                for tail in ["", "    assert value\n"] {
                    let source = format!(
                        "{scope} sample(flag, other, ready, value, state):\n    if flag and value == state.first and ready:\n        assert value\n    elif flag:\n        assert value\n    if {outer}:\n        if value == state.first:\n            assert value\n        elif value == state.second:\n            if {inner}:\n                assert value\n            else:\n                assert value\n{tail}"
                    );
                    assert_matches_cpython_versions(
                        &source,
                        "nested_erased_elif_footer.py",
                        &LEVELS,
                        &[PythonVersion::Py312],
                    );
                }
            }
        }
    }
}

#[test]
fn erased_binary_elif_arms_preserve_original_scope_footer() {
    for scope in ["def", "async def"] {
        for prefix in ["", "        value = operation(value)\n"] {
            for tail in [
                "",
                "    elif extra:\n        operation(value)\n",
                "    else:\n        raise Error\n",
            ] {
                let source = format!(
                    "{scope} sample(flag, other, first, before, inner, extra, value):\n    if flag:\n        if first:\n            assert value\n    elif other:\n{prefix}        if before:\n            assert value\n        if inner:\n            assert value\n        else:\n            assert value\n{tail}"
                );
                assert_matches_cpython_versions(
                    &source,
                    "erased_elif_footer.py",
                    &LEVELS,
                    &[PythonVersion::Py312],
                );
            }
        }
    }
}

#[test]
fn runtime_loop_break_preserves_nested_cleanup_borrowing() {
    for condition in ["state.active", "state.active and ready(items)"] {
        for guard in ["state.done", "state.done or state.error"] {
            for handler in ["pass", "recover(problem)"] {
                for cleanup in [
                    "cleanup(items)",
                    "try:\n            value = items\n        finally:\n            cleanup(items)",
                ] {
                    for handler_type in ["Error", "(Error, Other)", "errors.Error"] {
                        let source = format!(
                            "def sample(items, state):\n    try:\n        while {condition}:\n            if {guard}:\n                break\n            operation(items)\n    except {handler_type} as problem:\n        {handler}\n    finally:\n        {cleanup}\n"
                        );
                        assert_matches_cpython(&source, "runtime_break_cleanup.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn erased_suffix_after_await_preserves_scope_footer() {
    for prefix in [
        "values = [item for item in items]",
        "values = {item for item in items}",
        "values = {item: item for item in items}",
        "values = prepare(items)",
    ] {
        for awaited in ["operation(values)", "pending"] {
            for suffix in [
                "assert values\n",
                "assert values\n        assert other\n",
                "42\n",
            ] {
                let source = format!(
                    "async def sample(flag, other, items, pending):\n    if other:\n        assert items\n    if flag:\n        {prefix}\n        await {awaited}\n        {suffix}"
                );
                assert_matches_cpython_versions(
                    &source,
                    "await_erased_suffix.py",
                    &LEVELS,
                    &[PythonVersion::Py312],
                );
            }
        }
    }
}

#[test]
fn completed_comprehension_before_await_preserves_scope_footer() {
    for prefix in [
        "values = [item for item in items]",
        "values = {item for item in items}",
        "values = {item: item for item in items}",
        "values = prepare(items)",
    ] {
        for awaited in ["operation(values)", "pending"] {
            for guard in ["", "    if other:\n        assert items\n"] {
                let source = format!(
                    "async def sample(flag, other, items, pending):\n{guard}    if flag:\n        {prefix}\n        await {awaited}\n"
                );
                assert_matches_cpython_versions(
                    &source,
                    "comprehension_await_footer.py",
                    &LEVELS,
                    &[PythonVersion::Py312],
                );
            }
        }
    }
}

#[test]
fn raising_guard_preserves_awaited_recovery_cleanup_entry() {
    for handler in ["pass", "value = problem"] {
        for work in ["value = await operation(items)", "await operation(items)"] {
            for cleanup in ["await cleanup(items)", "cleanup(items)"] {
                for recovery in ["Other", "(Other, RecoveryError)"] {
                    let source = format!(
                        "async def sample(items, state):\n    try:\n        if state.ready:\n            raise Error\n    except Error as problem:\n        {handler}\n    finally:\n        try:\n            {work}\n        except {recovery}:\n            value = None\n        {cleanup}\n    finish(items)\n"
                    );
                    assert_matches_cpython(&source, "awaited_recovery_cleanup.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn erased_manager_guard_chain_preserves_exit_predecessors() {
    for scope in ["def", "async def"] {
        for prefix in [
            "",
            "        assert value\n        value = operation(value)\n        assert other\n",
        ] {
            for alternate in [
                "",
                "        elif value is not None:\n            assert flag\n            assert other\n",
                "        elif value is not None:\n            assert flag\n        else:\n            assert other\n",
            ] {
                let source = format!(
                    "{scope} sample(flag, other, value, manager):\n    with manager:\n{prefix}        if other:\n            assert value\n{alternate}        if flag:\n            raise Error(value)\n"
                );
                assert_matches_cpython_versions(
                    &source,
                    "erased_manager_guard_chain.py",
                    &LEVELS,
                    &[PythonVersion::Py312],
                );
            }
        }
    }
}

#[test]
fn erased_guard_before_raise_preserves_manager_exit() {
    for scope in ["def", "async def"] {
        for nested in [false, true] {
            for prefix_test in ["other", "value == 1"] {
                for final_test in ["flag", "value is None"] {
                    let mut source = format!(
                        "{scope} sample(flag, other, value, manager):\n    with manager:\n        if {prefix_test}:\n            assert value\n        if {final_test}:\n            raise RuntimeError('failed')\n"
                    );
                    if nested {
                        source = format!(
                            "def enclosing(flag, other, value, manager):\n{}",
                            source
                                .replace("sample(flag, other, value, manager)", "sample()")
                                .lines()
                                .fold(String::new(), |mut result, line| {
                                    writeln!(result, "    {line}").unwrap();
                                    result
                                })
                        );
                    }
                    assert_matches_cpython_versions(
                        &source,
                        "erased_manager_raise_exit.py",
                        &LEVELS,
                        &[PythonVersion::Py312],
                    );
                }
            }
        }
    }
}

#[test]
fn protected_iterator_work_ignores_erased_assertions() {
    for condition in ["flag", "flag == other"] {
        for work in ["value = item", "value = item.field[0]", "consume(item)"] {
            for (prefix, suffix) in [
                ("assert item\n", ""),
                ("", "assert other\n"),
                ("assert item\n", "assert other\n"),
            ] {
                let mut source = format!(
                    "def sample(items, flag, other):\n    try:\n        for item in items:\n            if {condition}:\n"
                );
                for line in format!("{prefix}{work}\n{suffix}").lines() {
                    writeln!(source, "                {line}").unwrap();
                }
                source.push_str(
                    "            else:\n                assert other\n    finally:\n        finish()\n",
                );
                assert_matches_cpython(&source, "protected_iterator_erased_work.py", &LEVELS);
            }
        }
    }
}

#[test]
fn protected_iterator_erased_join_preserves_handler_owner() {
    for scope in ["def", "async def"] {
        for handler in [
            "    except Error as error:\n        recover(error)\n        raise\n",
            "    except Error:\n        recover()\n",
        ] {
            for successor in [
                "            consume(item)\n",
                "            if other:\n                consume(item)\n",
                "            with inner():\n                consume(item)\n",
            ] {
                let source = format!(
                    "{scope} sample(items, flag, other):\n    try:\n        for item in items:\n            if flag:\n                assert other\n            else:\n                assert item\n{successor}{handler}    finally:\n        finish()\n"
                );
                assert_matches_cpython(&source, "protected_iterator_handler_join.py", &LEVELS);
            }
        }
    }
}

#[test]
fn protected_iterator_erased_join_preserves_conditional_successor() {
    for scope in ["def", "async def"] {
        for target in ["item", "(item, part)"] {
            for guard in ["flag", "item is not None", "item.ready"] {
                for successor in [
                    "            if other:\n                consume(item)\n",
                    "            with inner():\n                consume(item)\n",
                ] {
                    let source = format!(
                        "{scope} sample(items, flag, other):\n    try:\n        for {target} in items:\n            if {guard}:\n                assert other\n            else:\n                assert item\n{successor}    finally:\n        finish()\n"
                    );
                    assert_matches_cpython(&source, "protected_iterator_join.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn conditional_prefix_preserves_guarded_finalizer_manager_exit() {
    for guard in [
        "flag",
        "flag == 'ready'",
        "predicate(flag)",
        "not predicate(flag)",
    ] {
        for work in ["prepare()", "value = prepare()"] {
            for cleanup in [
                "            if predicate():\n                for item in items[:-1]:\n                    if not active(item):\n                        cleanup(item)\n            finish()\n",
                "            finish()\n",
            ] {
                for suffix in ["assert other", "pass"] {
                    let source = format!(
                        "def sample(flag, other, items):\n    with manager():\n        if {guard}:\n            {work}\n        try:\n            consume()\n        finally:\n{cleanup}        {suffix}\n"
                    );
                    assert_matches_cpython(&source, "conditional_prefix_finalizer.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn comprehension_call_prefix_preserves_manager_exit() {
    for comprehension in [
        "[item for item in items if predicate(item)]",
        "{part for item in items for part in item}",
        "{item: transform(item) for item in items}",
    ] {
        for call in [
            format!("prepare({comprehension})"),
            format!("prepare(other, values={comprehension})"),
            format!("worker.prepare(other, values={comprehension}, option=other)"),
        ] {
            for suffix in ["assert other", "pass"] {
                let source = format!(
                    "def sample(items, value, other):\n    with manager():\n        value = {call}\n        try:\n            assert value\n        finally:\n            cleanup()\n        {suffix}\n"
                );
                assert_matches_cpython(&source, "comprehension_call_prefix.py", &LEVELS);
            }
        }
    }
}

#[test]
fn completed_comprehension_prefix_preserves_manager_exit() {
    for comprehension in [
        "[item for item in items if predicate(item)]",
        "{(part, item.field) for item in items for part in item.parts}",
        "{part: item for item in items for part in item}",
    ] {
        for body in ["assert value", "observe(value)"] {
            for suffix in ["assert other", "pass"] {
                let source = format!(
                    "def sample(items, value, other):\n    with manager():\n        value = {comprehension}\n        try:\n            {body}\n        finally:\n            cleanup()\n        {suffix}\n"
                );
                assert_matches_cpython(&source, "completed_comprehension_prefix.py", &LEVELS);
            }
        }
    }
}

#[test]
fn completed_try_finalizer_preserves_erased_branch_join() {
    for scope in ["def", "async def"] {
        for prefix in ["", "        if value:\n            prepare()\n"] {
            for recovery in [
                "        finally:\n            finish()\n            observe()\n",
                "        except errors.Error:\n            recover()\n            observe()\n",
                "        except Error:\n            recover()\n        finally:\n            finish()\n",
            ] {
                let source = format!(
                    "{scope} sample(flag, value, other):\n    try:\n        assert value\n        if flag:\n            assert value\n        else:\n            assert other\n    finally:\n{prefix}        try:\n            cleanup()\n{recovery}"
                );
                assert_matches_cpython(&source, "completed_try_finalizer.py", &LEVELS);
            }
        }
    }
}

#[test]
fn attribute_finalizer_entry_preserves_erased_branch_join() {
    for scope in ["def", "async def"] {
        for value in ["other.field", "other.field.value", "prepare(other).field"] {
            for continuation in ["", "        observe()\n"] {
                for cleanup in [
                    "        cleanup()\n",
                    "        if value is not None:\n            cleanup()\n        finish()\n",
                ] {
                    let source = format!(
                        "{scope} sample(flag, value, other):\n    try:\n        assert value\n        if flag:\n            assert value\n        else:\n            assert other\n{continuation}    finally:\n        value = {value}\n{cleanup}"
                    );
                    assert_matches_cpython(&source, "attribute_finalizer_entry.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn guarded_finalizer_loop_preserves_erased_branch_join() {
    for scope in ["def", "async def"] {
        for iterable in ["items", "prepare(items)", "value.items"] {
            for work in ["consume(item)", "value = item"] {
                for inside_guard in [false, true] {
                    let finalizer = if inside_guard {
                        format!(
                            "        if value is not None:\n            value = None\n            for item in {iterable}:\n                {work}\n"
                        )
                    } else {
                        format!(
                            "        if value:\n            cleanup()\n        for item in {iterable}:\n            {work}\n"
                        )
                    };
                    let source = format!(
                        "{scope} sample(flag, value, other, items):\n    try:\n        assert value\n        if flag:\n            assert value\n        else:\n            assert other\n    finally:\n{finalizer}"
                    );
                    assert_matches_cpython(&source, "guarded_finalizer_loop.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn nested_guarded_finalizer_preserves_erased_branch_join() {
    for (scope, cleanup) in [("def", "finish()"), ("async def", "await finish()")] {
        for condition in ["flag", "not predicate(value)", "flag or other"] {
            for work in ["cleanup()", "value = other"] {
                for alternate in ["", "            else:\n                recover()\n"] {
                    let source = format!(
                        "{scope} sample(flag, value, other):\n    try:\n        assert value\n        if flag:\n            assert value\n        else:\n            assert other\n    finally:\n        if value is not None:\n            if {condition}:\n                {work}\n{alternate}            {cleanup}\n"
                    );
                    assert_matches_cpython(&source, "nested_guarded_finalizer.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn guarded_finalizer_store_preserves_erased_branch_join() {
    for (scope, cleanup) in [("def", "cleanup()"), ("async def", "await cleanup()")] {
        for target in ["value", "value.field", "value[0]", "left, right"] {
            for expression in ["other", "prepare(other)"] {
                let source = format!(
                    "{scope} sample(flag, value, other):\n    try:\n        assert value\n        if flag == 'ready':\n            assert value\n        else:\n            assert other\n    finally:\n        if value is not None:\n            {target} = {expression}\n        {cleanup}\n"
                );
                assert_matches_cpython(&source, "guarded_finalizer_store.py", &LEVELS);
            }
        }
    }
}

#[test]
fn nested_manager_preserves_original_scope_footer() {
    for scope in ["def", "async def"] {
        for condition in ["other", "value in (1, 2)"] {
            for manager in ["manager", "open_manager(value)"] {
                for tail in [
                    "",
                    "    else:\n        value = operation(value)\n        with manager as item:\n            value = operation(item)\n            assert value\n",
                ] {
                    let source = format!(
                        "{scope} sample(flag, other, value, manager):\n    if flag:\n        if other:\n            assert value\n        if {condition}:\n            with {manager}:\n                operation(value)\n{tail}"
                    );
                    assert_matches_cpython_versions(
                        &source,
                        "nested_manager_scope_footer.py",
                        &LEVELS,
                        &[PythonVersion::Py312],
                    );
                }
            }
        }
    }
}

#[test]
fn completed_prefix_preserves_nested_iterator_footer() {
    for scope in ["def", "async def"] {
        for prefix in [
            "            value = operation(value)\n",
            "            for prior in items:\n                assert prior\n",
            "            for prior in items:\n                value = operation(prior)\n                if value is None:\n                    assert prior\n                else:\n                    assert value\n                    operation(prior)\n",
        ] {
            for inner_else in ["", "        else:\n            assert value\n"] {
                for outer_else in ["", "    else:\n        assert value\n"] {
                    let source = format!(
                        "{scope} sample(flag, other, value, items):\n    if not other:\n        assert value\n    if flag:\n        value = operation(value)\n        if other:\n{prefix}            for item in items:\n                assert item\n                operation(item)\n                finish(item)\n{inner_else}{outer_else}"
                    );
                    assert_matches_cpython_versions(
                        &source,
                        "completed_nested_iterator_footer.py",
                        &LEVELS,
                        &[PythonVersion::Py312],
                    );
                }
            }
        }
    }
}

#[test]
fn nested_finally_guarded_recovery_preserves_conditional_call_borrowing() {
    for body in [
        "pass",
        "if state is None:\n            return",
        "if state is None:\n            return 1",
    ] {
        for inner in ["pass", "operation()", "await operation()"] {
            for handler in [
                "pass",
                "raise",
                "await recover()\n                    raise",
            ] {
                for cleanup in ["cleanup(items)", "cleanup(items if state else other)"] {
                    let source = format!(
                        "async def sample(items, state):\n    try:\n        {body}\n    finally:\n        try:\n            if state is not None:\n                try:\n                    {inner}\n                except Error:\n                    {handler}\n        except Other:\n            raise\n        finally:\n            {cleanup}\n"
                    );
                    assert_matches_cpython(&source, "nested_finally_guard.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn nested_iterator_erased_else_preserves_scope_footer() {
    for scope in ["def", "async def"] {
        for condition in ["flag", "flag is None", "not flag"] {
            for body in ["operation(item)", "value = operation(item)"] {
                for inner_else in ["", "        else:\n            assert value\n"] {
                    for outer_else in ["", "    else:\n        assert value\n"] {
                        let source = format!(
                            "{scope} sample(flag, other, value, items):\n    if not other:\n        assert value\n    if {condition}:\n        if other:\n            for item in items:\n                assert item\n                {body}\n{inner_else}{outer_else}"
                        );
                        assert_matches_cpython_versions(
                            &source,
                            "nested_iterator_erased_else.py",
                            &LEVELS,
                            &[PythonVersion::Py312],
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn nested_live_iterator_preserves_original_return_footer() {
    for scope in ["def", "async def"] {
        for prefix in [
            "",
            "    if not other:\n        assert value\n",
            "    if other:\n        assert value\n",
        ] {
            for body in [
                "operation(item)",
                "value = operation(item)",
                "assert item\n                operation(item)",
                "operation(item)\n                assert item",
                "assert item\n                value = operation(item)\n                assert value",
                "assert item\n                tools.check.equal(item.value, value)",
            ] {
                let source = format!(
                    "{scope} sample(flag, other, value, items):\n{prefix}    if flag:\n        if other:\n            for item in items:\n                {body}\n"
                );
                assert_matches_cpython_versions(
                    &source,
                    "nested_live_iterator_footer.py",
                    &LEVELS,
                    &[PythonVersion::Py312],
                );
            }
        }
    }
}

#[test]
fn erased_chained_boolean_preserves_scope_exit_order() {
    for scope in ["def", "async def"] {
        for condition in [
            "left or lower < value < upper",
            "left == 3 or lower < value < upper",
            "values[0] == 3 or values[0] == values[1] > 0",
            "left or lower < value < upper < last",
            "left and lower < value < upper",
            "lower < value < upper or left",
        ] {
            for tail in ["assert value", "pass"] {
                let source = format!(
                    "{scope} sample(left, lower, value, upper, last, values):\n    if {condition}:\n        {tail}\n"
                );
                assert_matches_cpython_versions(
                    &source,
                    "erased_chain_exit_order.py",
                    &LEVELS,
                    &[PythonVersion::Py312],
                );
            }
        }
    }
}

#[test]
fn nested_erased_else_preserves_original_return_footer() {
    for scope in ["def", "async def"] {
        for outer in ["flag", "flag is None", "flag == 1"] {
            for inner in ["other", "other is None", "other == 2"] {
                for prefix in [
                    "",
                    "        if prior:\n            assert value\n",
                    "        if prior:\n            assert value\n        else:\n            assert value\n",
                    "        if prior:\n            assert value\n        else:\n            assert value\n        if prior:\n            assert value\n",
                ] {
                    let source = format!(
                        "{scope} sample(flag, other, prior, value):\n    if {outer}:\n        assert value\n    else:\n{prefix}        if {inner}:\n            assert value\n        else:\n            assert operation(value)\n"
                    );
                    assert_matches_cpython(&source, "nested_erased_else_footer.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn async_comprehension_preserves_preceding_erased_loop_locations() {
    assert_matches_cpython(
        "async def sample(items, left, right, stream):\n    for item in items:\n        if left:\n            assert item\n        if right is not None:\n            assert operation(item)\n    values = [await convert(part) async for part in stream]\n",
        "async_comprehension_loop_location.py",
        &LEVELS,
    );
    for condition in ["right", "right is None", "right is not None"] {
        for value in ["part", "await convert(part)"] {
            for expression in [
                format!("[{value} async for part in stream]"),
                format!("{{{value} async for part in stream}}"),
                format!("{{part: {value} async for part in stream}}"),
            ] {
                let source = format!(
                    "async def sample(items, left, right, stream):\n    for item in items:\n        if left:\n            assert item\n        if {condition}:\n            assert operation(item)\n    values = {expression}\n"
                );
                assert_matches_cpython(&source, "async_comprehension_loop_location.py", &LEVELS);
            }
        }
    }
}

#[test]
fn nested_finally_preserves_runtime_loop_borrow_entry() {
    for condition in ["state.ready", "not await operation(items)"] {
        for inner in ["pass", "await operation()"] {
            for handler in ["raise", "await recover()\n                    raise"] {
                let source = format!(
                    "async def sample(items, state):\n    try:\n        while {condition}:\n            pass\n        completed = True\n    except Error as problem:\n        if state.ready:\n            raise\n    finally:\n        try:\n            if state is not None:\n                try:\n                    {inner}\n                except RecoveryError:\n                    {handler}\n        except Other:\n            report(items)\n        finally:\n            cleanup(items if state else other)\n"
                );
                assert_matches_cpython(&source, "nested_finally_entry.py", &LEVELS);
            }
        }
    }
}

#[test]
fn async_iterator_return_else_preserves_finally_borrow_boundary() {
    for yielded in ["", "            yield item\n"] {
        for suffix in [
            "",
            "        operation()\n",
            "        while True:\n            if state.ready:\n                break\n",
        ] {
            for cleanup in [
                "result = cleanup(items)",
                "result = getattr(items, 'finish', None)",
            ] {
                let source = format!(
                    "async def sample(items, state):\n    try:\n        async for item in items:\n{yielded}            if state.ready:\n                break\n        else:\n            return\n{suffix}    finally:\n        {cleanup}\n"
                );
                assert_matches_cpython(&source, "iterator_finally_borrow.py", &LEVELS);
            }
        }
    }
}

#[test]
fn nested_terminal_recovery_preserves_borrow_boundary() {
    for outer_name in ["", " as outer"] {
        for inner_name in ["", " as inner"] {
            let source = format!(
                "async def sample(value):\n    try:\n        await work()\n    except RuntimeError{outer_name}:\n        try:\n            operation()\n        except ValueError{inner_name}:\n            try:\n                operation()\n            except KeyError:\n                raise\n            pass\n        else:\n            pass\n        pass\n    else:\n        pass\n    if value.flag:\n        pass\n"
            );
            let body = source.strip_prefix("async def sample(value):\n").unwrap();
            for (prefix, indentation, suffix) in [
                ("", "", ""),
                (
                    "    match value:\n        case [item]:\n",
                    "        ",
                    "        case _:\n            pass\n",
                ),
                (
                    "    match value:\n        case item if ready:\n",
                    "        ",
                    "        case _:\n            pass\n",
                ),
                (
                    "    try:\n        enclosing_operation()\n    except LookupError:\n",
                    "    ",
                    "",
                ),
                (
                    "    try:\n        enclosing_operation()\n    except LookupError as enclosing:\n",
                    "    ",
                    "",
                ),
                (
                    "    try:\n        enclosing_operation()\n    except* LookupError as enclosing:\n",
                    "    ",
                    "",
                ),
            ] {
                let mut enclosed = format!("async def sample(value):\n{prefix}");
                for line in body.lines() {
                    writeln!(enclosed, "{indentation}{line}").unwrap();
                }
                enclosed.push_str(suffix);
                assert_matches_cpython(&enclosed, "nested_recovery_borrow.py", &LEVELS);
            }
        }
    }
}

#[test]
fn nested_return_before_erased_assertions_preserves_original_footer() {
    assert_matches_cpython(
        "def sample(flag, outer, inner):\n    if flag:\n        assert flag\n    if outer:\n        if inner:\n            return\n        assert flag\n",
        "nested_return_suffix.py",
        &LEVELS,
    );
    for scope in ["def", "async def"] {
        for prefix in ["", "    if flag:\n        assert flag\n"] {
            for outer in ["outer", "not outer", "outer.value"] {
                for returned in ["return", "return None"] {
                    for assertion in ["assert flag", "assert operation(flag)"] {
                        let source = format!(
                            "{scope} sample(flag, outer, inner):\n{prefix}    if {outer}:\n        assert flag\n        if inner:\n            {returned}\n            {assertion}\n        {assertion}\n    {assertion}\n"
                        );
                        assert_matches_cpython(&source, "nested_return_suffix.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn nested_terminal_return_preserves_original_footer() {
    assert_matches_cpython(
        "def sample(flag, outer, inner):\n    if flag:\n        assert flag\n    if outer:\n        if inner:\n            return\n",
        "nested_return_footer.py",
        &LEVELS,
    );
    for scope in ["def", "async def"] {
        for prefix in ["", "    if flag:\n        assert flag\n"] {
            for outer in ["outer", "not outer", "outer.value"] {
                for inner in ["inner", "not inner", "inner == 3"] {
                    for returned in ["return", "return None"] {
                        let source = format!(
                            "{scope} sample(flag, outer, inner):\n{prefix}    if {outer}:\n        if {inner}:\n            {returned}\n"
                        );
                        assert_matches_cpython(&source, "nested_return_footer.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn handler_loop_else_keeps_original_borrow_boundary() {
    for protected in ["pass", "operation()"] {
        for normal in ["pass", "operation()"] {
            for condition in ["finished()", "finished() < limit", "not finished()"] {
                let source = format!(
                    "def sample(value, limit):\n    try:\n        {protected}\n    except RuntimeError:\n        while True:\n            if {condition}:\n                break\n    else:\n        {normal}\n    if value is not None:\n        operation(value)\n"
                );
                assert_matches_cpython(&source, "handler_loop_else_borrow.py", &LEVELS);
            }
        }
    }
}

#[test]
fn terminal_subscript_iterator_keeps_return_location() {
    assert_matches_cpython(
        "async def sample(flag, items):\n    if flag:\n        assert await prepare()\n    if not flag:\n        for item in items.groups[0].values[1:]:\n            assert await operation(item)\n",
        "terminal_subscript_return.py",
        &LEVELS,
    );
    for scope in ["def", "async def"] {
        for prefix in ["", "    if flag:\n        assert value\n"] {
            for condition in ["flag", "not flag"] {
                for iterator in [
                    "items[0]",
                    "items[1:]",
                    "items[1:4]",
                    "items[::2]",
                    "items[index]",
                    "items[index:end:step]",
                    "items.groups[0].values[1:]",
                ] {
                    for body in ["assert item", "operation(item)"] {
                        let source = format!(
                            "{scope} sample(flag, value, items, index, end, step):\n{prefix}    if {condition}:\n        for item in {iterator}:\n            {body}\n"
                        );
                        assert_matches_cpython(&source, "terminal_subscript_iterator.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn erased_prefix_before_terminal_iterator_keeps_return_location() {
    assert_matches_cpython(
        "async def sample(unused, value):\n    if value.kind == 'first':\n        assert operation(item, (1, 2, 3)) > 7\n    if value.kind == 'last':\n        for item in ((1, 2, 3), (4, 5, 6), (7, 8, 9)):\n            assert operation(value, item) > 7\n",
        "terminal_iterator_return_nested_tuple.py",
        &LEVELS,
    );
    for scope in ["def", "async def"] {
        for prefix in [
            "",
            "    if other:\n        assert other\n",
            "    if flag.name == 'first':\n        assert other\n",
        ] {
            for condition in ["flag", "flag.name == 'last'"] {
                for iterator in [
                    "items",
                    "(1, 2, 3)",
                    "((1, 2), (3, 4))",
                    "[(1, 2), (3, 4)]",
                    "(((1,),), ((2,),))",
                    "((None, True, 'text'), (False, b'bytes', ...))",
                ] {
                    for body in ["assert item", "operation(item)"] {
                        let source = format!(
                            "{scope} sample(flag, other, items):\n{prefix}    if {condition}:\n        for item in {iterator}:\n            {body}\n"
                        );
                        assert_matches_cpython(&source, "terminal_iterator_return.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn iterator_break_copies_small_literal_tuple_return() {
    for (scope, iterator, prefix) in [
        ("def", "for", ""),
        ("def", "for", "    yield flag\n"),
        ("async def", "for", ""),
        ("async def", "async for", ""),
    ] {
        for value in ["()", "(1,)", "('value',)", "(None,)"] {
            for preceding in [
                "",
                "    try:\n        prepare()\n    except Exception:\n        return ()\n",
            ] {
                let source = format!(
                    "{scope} sample(values, flag):\n{prefix}{preceding}    {iterator} value in values:\n        if flag:\n            break\n        operation(value)\n    return {value}\n"
                );
                assert_matches_cpython(&source, "small_tuple_break.py", &LEVELS);
            }
        }
    }
}

#[test]
fn nested_or_match_exit_inherits_guard_location() {
    for first in ["Item()", "[Item()]"] {
        for guard in ["flag", "value is not None"] {
            for value in ["value", "operation(value)"] {
                let source = format!(
                    "def sample(node, flag):\n    match node:\n        case Header():\n            match node:\n                case (\n                    First(item={first}, value=value)\n                    | Second(item=Item(), value=value)\n                ) if {guard}:\n                    return {value}\n        case Other():\n            return node\n    return None\n"
                );
                assert_matches_cpython(
                    &source,
                    &format!("nested_or_match_guard_{}.py", guard.len()),
                    &LEVELS,
                );
            }
        }
    }
}

#[test]
fn nested_awaited_manager_finalizer_owner() {
    for condition in ["started", "not started", "not started and ready(source)"] {
        for cleanup in [
            "reset()",
            "state.reset(token)",
            "resource.reset()",
            "reset(token)",
        ] {
            let source = format!(
                "async def sample(source, token, resource):\n    started = False\n    try:\n        try:\n            with manager():\n                started = True\n                return await source\n        finally:\n            {cleanup}\n    finally:\n        if {condition}:\n            finish()\n"
            );
            assert_matches_cpython(&source, "awaited_manager_finalizer.py", &LEVELS);
        }
        for handler in ["ValueError", "ValueError as error"] {
            let source = format!(
                "async def sample(source, token):\n    started = False\n    try:\n        try:\n            with manager():\n                started = True\n                try:\n                    return await source\n                except {handler}:\n                    raise\n        finally:\n            state.reset(token)\n    except LookupError:\n        raise\n    finally:\n        if {condition}:\n            finish()\n"
            );
            assert_matches_cpython(&source, "awaited_try_manager_finalizer.py", &LEVELS);
        }
    }
    for cleanup in ["reset()", "reset(token)"] {
        let source = format!(
            "async def sample(source, token):\n    started = False\n    try:\n        try:\n            with manager():\n                started = True\n                try:\n                    return await source\n                except ValueError as error:\n                    raise\n        finally:\n            {cleanup}\n    except LookupError as error:\n        raise\n    finally:\n        if started is None:\n            finish()\n"
        );
        assert_matches_cpython(&source, "awaited_manager_none_control.py", &LEVELS);
    }
}

#[test]
fn nested_named_handler_return_truth_finalizer() {
    for scope in ["def", "async def"] {
        for handler in ["Exception", "Exception as error"] {
            for expression in ["result", "operation(result)", "{'value': result}"] {
                for condition in ["resource", "not resource"] {
                    for body in ["pass", "finish(resource)"] {
                        let source = format!(
                            "{scope} sample(resource, result):\n    try:\n        operation()\n    except {handler}:\n        try:\n            operation()\n        except ValueError:\n            return {expression}\n    finally:\n        if {condition}:\n            {body}\n"
                        );
                        assert_matches_cpython(&source, "nested_named_handler_truth.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn nested_manager_return_finalizer_owner() {
    for managers in 1..=6 {
        for expression in ["result", "operation(result)"] {
            for (condition, body) in [
                ("worker is not None", "pass"),
                (
                    "worker is not None and worker.poll() is None",
                    "finish(worker)",
                ),
            ] {
                let mut source =
                    "def sample(manager, worker, result, flag):\n    try:\n".to_owned();
                let mut indent = "        ".to_owned();
                for index in 0..managers {
                    writeln!(source, "{indent}with manager() as item{index}:").unwrap();
                    indent.push_str("    ");
                }
                writeln!(source, "{indent}if flag:\n{indent}    operation()\n{indent}return {expression}\n    finally:\n        if {condition}:\n            {body}").unwrap();
                assert_matches_cpython(&source, "nested_manager_finalizer.py", &LEVELS);
            }
        }
    }
}

#[test]
fn fused_stores_preserve_copied_handler_finally_marker_owner() {
    for stores in 2..=5 {
        for handler in ["Exception", "Exception as error"] {
            for condition in [
                "value is None",
                "value is not None",
                "value is not None and not done",
            ] {
                let mut assignments = String::new();
                for index in 0..stores {
                    writeln!(assignments, "            self.field{index} = value").unwrap();
                }
                let source = format!(
                    "async def sample(self, items, flag):\n    for item in items:\n        value = None\n        done = False\n        try:\n            value = await acquire()\n{assignments}            done = True\n            return\n        except {handler}:\n            if flag:\n                raise\n        finally:\n            if {condition}:\n                await release(value)\n        await advance()\n"
                );
                assert_matches_cpython(&source, "fused_handler_finally_prefix.py", &LEVELS);
            }
        }
    }
}

#[test]
fn fused_stores_preserve_finally_none_marker_owner() {
    for scope in ["def", "async def"] {
        for stores in 2..=5 {
            for condition in ["value is None", "value is not None"] {
                let mut assignments = String::new();
                for index in 0..stores {
                    writeln!(assignments, "            self.field{index} = value").unwrap();
                }
                let source = format!(
                    "{scope} sample(self, value, items):\n    for item in items:\n        try:\n{assignments}            return\n        finally:\n            if {condition}:\n                operation(value)\n"
                );
                assert_matches_cpython(&source, "fused_stores_finally_none.py", &LEVELS);
            }
        }
    }
}

#[test]
fn terminal_manager_try_break_retains_owner() {
    assert_matches_cpython(
        "def sample(manager):\n    try:\n        with manager:\n            try:\n                while True:\n                    try:\n                        operation()\n                        break\n                    except ValueError:\n                        continue\n            except Exception:\n                finish()\n                raise\n    except OSError:\n        raise ValueError() from None\n    finally:\n        finish()\n",
        "terminal_manager_try_break_outer_finally.py",
        &LEVELS,
    );
    for (scope, manager) in [
        ("def", "with"),
        ("async def", "with"),
        ("async def", "async with"),
    ] {
        for condition in ["True", "flag"] {
            for handler in ["Exception", "Exception as error"] {
                for action in ["operation()", "value = 1"] {
                    for nested in [false, true] {
                        let body = if nested {
                            format!(
                                "                try:\n                    {action}\n                    break\n                except OtherError:\n                    continue\n"
                            )
                        } else {
                            format!("                {action}\n                break\n")
                        };
                        let source = format!(
                            "{scope} sample(manager, flag):\n    {manager} manager:\n        try:\n            while {condition}:\n{body}        except {handler}:\n            raise\n"
                        );
                        assert_matches_cpython(&source, "terminal_manager_try_break.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn reraising_finally_preserves_coroutine_try_join() {
    for condition in ["flag", "flag is not None"] {
        for handler_prefix in ["", "                value = 1\n"] {
            for exception in ["Error", "errors.Error"] {
                for body in [
                    "                operation()\n",
                    "                async with manager():\n                    result = await operation()\n",
                ] {
                    for cleanup in ["cleanup()", "observe(value if flag else items)"] {
                        let source = format!(
                            "async def sample(flag, items):\n    if items:\n        observe()\n    try:\n        if {condition}:\n            try:\n{body}            except {exception}:\n{handler_prefix}                raise\n            finally:\n                try:\n                    {cleanup}\n                except Other:\n                    pass\n    except Outer:\n        failed = True\n    observe()\n"
                        );
                        assert_matches_cpython(&source, "reraising_finally_join.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn completed_generator_prefix_preserves_normal_finalizer_exit() {
    for generator in [
        "(item for item in values)",
        "(item.value for item in source(values) if item)",
        "(item if other else value for item in values)",
    ] {
        for callable in ["observe", "sink.consume"] {
            for prefix in ["prepare(value)", "prepared = prepare(value)"] {
                for assertion in ["assert result", "assert all(result for item in values)"] {
                    let source = format!(
                        "def sample(manager, value, values, other):\n    with manager():\n        {callable}('tag', {generator})\n        {prefix}\n        try:\n            result = process(value)\n        finally:\n            cleanup(value)\n        {assertion}\n"
                    );
                    assert_matches_cpython(&source, "completed_generator_finalizer.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn nested_populated_conditions_preserve_original_joins() {
    let indent = |body: &str| {
        body.lines().fold(String::new(), |mut indented, line| {
            writeln!(indented, "    {line}").unwrap();
            indented
        })
    };
    for condition in [
        "other and flag",
        "other or flag",
        "(first or other) and (flag or not second)",
    ] {
        for depth in [1, 2] {
            for prefix in ["", "if other:\n    total = update(values)\n"] {
                for successor in [
                    "observe(values)\n",
                    "return values\n",
                    "selected = values[flag]\nreturn selected\n",
                ] {
                    let mut branch = format!(
                        "if second:\n    if other:\n        raise Error()\n    result = values\nelif {condition}:\n    result = values\nelse:\n    result = source(values)\n    result = result.value\n"
                    );
                    for _ in 0..depth {
                        let arm =
                            format!("total = measure(values)\n{prefix}result = finish(total)\n");
                        branch = format!("if first:\n{}else:\n{}", indent(&arm), indent(&branch));
                    }
                    let body = format!(
                        "if flag:\n{}else:\n    assert flag\n{successor}",
                        indent(&branch)
                    );
                    let source = format!(
                        "def sample(flag, other, first, second, values):\n{}",
                        indent(&body)
                    );
                    assert_matches_cpython(&source, "nested_populated_join.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn completed_elif_arms_preserve_original_join() {
    for prefix in [
        "",
        "            if other:\n                total = update(values)\n",
        "            if other is None:\n                audit(total)\n",
    ] {
        for condition in [
            "other",
            "other and flag",
            "other or flag",
            "not other or flag",
        ] {
            for successor in [
                "    observe(values)\n",
                "    return values\n",
                "    selected = values[flag]\n    return selected\n",
            ] {
                let source = format!(
                    "def sample(flag, other, first, second, values):\n    if flag:\n        selected = values\n        if first:\n            assert first\n            total = measure(values)\n{prefix}            result = finish(total)\n        elif second:\n            assert second\n            if other:\n                raise Error()\n            result = values\n        elif {condition}:\n            result = values\n        else:\n            result = source(values)\n            result = result.value\n    else:\n        assert flag\n{successor}"
                );
                assert_matches_cpython(&source, "completed_elif_join.py", &LEVELS);
            }
        }
    }
}

#[test]
fn ordinary_completed_arms_preserve_erased_tail_joins() {
    for value in [
        "values",
        "[item for item, ready in values if ready]",
        "{item for item, ready in values if ready}",
        "{item: ready for item, ready in values if ready}",
        "tuple(item for item in values)",
    ] {
        for condition in ["other", "all(item > 0 for item in values)"] {
            for assertion in ["assert count", "assert all(count for item in values)"] {
                for successor in [
                    "    observe(values)\n",
                    "    return values\n",
                    "    selected = values[flag]\n    return selected\n",
                ] {
                    let source = format!(
                        "def sample(flag, other, values):\n    if flag:\n        size = measure(values)\n        assert size\n        if {condition}:\n            assert values\n        else:\n            items = {value}\n            count = measure(items) + 1\n            {assertion}\n    else:\n        assert flag\n{successor}"
                    );
                    assert_matches_cpython(&source, "ordinary_completed_arm_tail.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn enclosing_finalizers_preserve_nested_loop_joins() {
    let indent = |body: &str| {
        body.lines().fold(String::new(), |mut indented, line| {
            writeln!(indented, "    {line}").unwrap();
            indented
        })
    };
    for (manager, nested_loop) in [
        (Some("manager()"), false),
        (Some("manager()"), true),
        (Some("first_manager(), second_manager()"), true),
        (None, false),
    ] {
        for condition in ["other", "entry[0] == other"] {
            for successor in [
                "observe(values)\n",
                "for item in values:\n    assert item\n",
                "for item, extra, last in values:\n    assert item\n",
            ] {
                let conditional = format!(
                    "if flag:\n    assert entry\n    if {condition}:\n        assert flag\n    else:\n        assert other\nelse:\n    assert flag\n{successor}"
                );
                let mut body = format!("for entry in values:\n{}", indent(&conditional));
                if nested_loop {
                    body = format!("for row in values:\n{}", indent(&body));
                }
                body = format!("try:\n{}finally:\n    cleanup()\n", indent(&body));
                if let Some(manager) = manager {
                    body = format!("with {manager}:\n{}", indent(&body));
                }
                let source = format!("def sample(values, flag, other):\n{}", indent(&body));
                assert_matches_cpython(&source, "nested_finally_loop_join.py", &LEVELS);
            }
        }
    }
}

#[test]
fn completed_coroutine_arms_preserve_erased_outer_join() {
    for value in [
        "first",
        "first if other else second",
        "left() if other is None else right()",
        "(first if flag else second) if other else third()",
    ] {
        for assertion in ["assert value", "assert all(value for item in values)"] {
            for guard in ["flag", "flag is None"] {
                for suffix in [
                    "    observe(values)\n",
                    "    selected = values[flag]\n    return selected\n",
                    "    return factory(values=values if other else None)\n",
                ] {
                    let source = format!(
                        "async def sample(flag, other, first, second, values):\n    if {guard}:\n        if other:\n            assert values\n            observe(values)\n        else:\n            assert first\n            value = {value}\n            {assertion}\n    else:\n        assert flag\n{suffix}"
                    );
                    assert_matches_cpython(&source, "completed_coroutine_arm.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn completed_comprehensions_preserve_erased_conditional_joins() {
    for scope in ["def", "async def"] {
        for value in [
            "[item.value for item in values]",
            "{item.value for item in values}",
            "{item.name: item.value for item in values}",
        ] {
            for guard in ["flag", "flag == 1"] {
                for condition in ["other", "'key' in values"] {
                    for suffix in [
                        "    return values\n",
                        "    observe(values)\n",
                        "    selected = values[flag]\n    return selected\n",
                        "    flag += 1\n",
                        "    if other:\n        raise Error()\n",
                        "    return factory(values=values if flag else None)\n",
                    ] {
                        let source = format!(
                            "{scope} sample(flag, values, other):\n    if {guard}:\n        assert other\n        items = {value}\n        if {condition}:\n            assert items\n        else:\n            assert other\n    else:\n        assert flag\n{suffix}"
                        );
                        assert_matches_cpython(
                            &source,
                            "completed_comprehension_erased_join.py",
                            &LEVELS,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn erased_try_else_retains_conditional_manager_exit() {
    for managers in [
        "manager()",
        "manager() as chosen",
        "first_manager(), second_manager(), third_manager()",
    ] {
        for condition in ["flag", "flag == first"] {
            for (left, right) in [
                ("first", "second"),
                ("left()", "right()"),
                (
                    "factory(first, option=right())",
                    "factory(second, option=left())",
                ),
            ] {
                for assertions in [
                    "            assert value\n",
                    "            assert value\n            assert first\n",
                ] {
                    let source = format!(
                        "def sample(flag, first, second):\n    with {managers}:\n        try:\n            if {condition}:\n                value = {left}\n            else:\n                value = {right}\n        except Error as error:\n            assert check(error)\n        else:\n{assertions}"
                    );
                    assert_matches_cpython(&source, "erased_try_else_manager.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn completed_erased_iterators_preserve_ordinary_conditional_joins() {
    for scope in ["def", "async def"] {
        for iterable in ["values", "(first, second)", "operation()"] {
            for repeated in [false, true] {
                for ternary in [false, true] {
                    for condition in ["other", "other is None"] {
                        for successor in [
                            "    operation()\n",
                            "    return first\n",
                            "    if not flag:\n        assert other\n",
                        ] {
                            let stored = if ternary {
                                "        value = first if other else second\n"
                            } else {
                                ""
                            };
                            let second = if repeated {
                                format!(
                                    "        for item in {iterable}:\n            assert item\n"
                                )
                            } else {
                                String::new()
                            };
                            let source = format!(
                                "{scope} sample(flag, other, values, first, second, operation):\n    if flag:\n        for item in {iterable}:\n            assert item\n{stored}{second}        if {condition}:\n            assert first\n        else:\n            assert other\n    else:\n        assert flag\n{successor}"
                            );
                            assert_matches_cpython(
                                &source,
                                &format!(
                                    "completed_erased_iterator_{}_{}_{}_{}_{}_{}.py",
                                    scope.len(),
                                    iterable.len(),
                                    repeated,
                                    ternary,
                                    condition.len(),
                                    successor.len()
                                ),
                                &LEVELS,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn conditional_manager_contexts_preserve_erased_arm_continuations() {
    for (scope, manager) in [
        ("def", "with"),
        ("async def", "with"),
        ("async def", "async with"),
    ] {
        for callable in ["factory", "module.factory"] {
            for value in [
                "first if flag else second",
                "(first if flag else second) if other else None",
                "first if other else (second if flag else None)",
                "operation() if other else operation()",
                "first if other and flag else second",
                "first if other is None else second",
            ] {
                for binding in ["", " as chosen"] {
                    for stored_prefix in [false, true] {
                        let prefix = if stored_prefix {
                            format!("            value = {value}\n")
                        } else {
                            String::new()
                        };
                        let work = if scope == "def" {
                            "operation()"
                        } else {
                            "await awaitable"
                        };
                        let source = format!(
                            "{scope} sample(flag, other, first, second, operation, factory, module, awaitable):\n    try:\n        if other:\n            operation()\n        else:\n{prefix}            {manager} {callable}({value}){binding}:\n                {work}\n            if flag:\n                assert first\n            else:\n                assert other\n    finally:\n        operation()\n"
                        );
                        assert_matches_cpython(
                            &source,
                            &format!(
                                "conditional_manager_{}_{}_{}_{}_{}_{}.py",
                                scope.len(),
                                manager.len(),
                                callable.len(),
                                value.len(),
                                binding.len(),
                                stored_prefix
                            ),
                            &LEVELS,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn completed_conditional_prefixes_preserve_ordinary_erased_joins() {
    for scope in ["def", "async def"] {
        for value in [
            "first if other else second",
            "(first if flag else second) if other else None",
            "first if other else (second if flag else None)",
            "operation() if other else operation()",
            "first if other and flag else second",
            "first if other is None else second",
        ] {
            for repeated in [false, true] {
                for condition in ["other", "other is None"] {
                    for successor in [
                        "    operation(value)\n",
                        "    result = first if flag else second\n    return result\n",
                        "    return value\n",
                    ] {
                        let prefix = if repeated {
                            format!("        second = {value}\n        assert second\n")
                        } else {
                            String::new()
                        };
                        let source = format!(
                            "{scope} sample(flag, other, first, second, operation):\n    if flag:\n        value = {value}\n        assert value\n{prefix}        if {condition}:\n            assert first\n        else:\n            assert second\n    else:\n        assert flag\n{successor}"
                        );
                        assert_matches_cpython(
                            &source,
                            &format!(
                                "conditional_prefix_{}_{}_{}_{}_{}.py",
                                scope.len(),
                                value.len(),
                                repeated,
                                condition.len(),
                                successor.len()
                            ),
                            &LEVELS,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn generator_finalizer_calls_preserve_erased_arm_joins() {
    for (scope, awaited) in [("def", ""), ("async def", ""), ("async def", "await ")] {
        for callable in ["operation", "module.operation"] {
            for starred in ["", "*"] {
                for element in ["item", "module.operation(item)"] {
                    for condition in ["flag", "flag is None"] {
                        for store in ["", "value = "] {
                            for nested in [false, true] {
                                let protected = if nested {
                                    format!(
                                        "        if other:\n            operation()\n        else:\n            if {condition}:\n                assert items\n            else:\n                assert other\n"
                                    )
                                } else {
                                    format!(
                                        "        if {condition}:\n            assert items\n        else:\n            assert other\n"
                                    )
                                };
                                let source = format!(
                                    "{scope} sample(flag, other, module, operation, items):\n    try:\n{protected}    finally:\n        {store}{awaited}{callable}({starred}({element} for item in items))\n"
                                );
                                assert_matches_cpython(
                                    &source,
                                    &format!(
                                        "generator_finalizer_{}_{}_{}_{}_{}_{}_{}_{}.py",
                                        scope.len(),
                                        awaited.len(),
                                        callable.len(),
                                        starred.len(),
                                        element.len(),
                                        condition.len(),
                                        store.len(),
                                        nested
                                    ),
                                    &LEVELS,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn completed_ternary_prefixes_preserve_nested_erased_arm_joins() {
    for (scope, manager) in [
        ("def", ""),
        ("def", "with"),
        ("async def", ""),
        ("async def", "with"),
        ("async def", "async with"),
    ] {
        for value in [
            "first if other else second",
            "(first if flag else second) if other else None",
            "first if other else (second if flag else None)",
            "operation() if other else operation()",
            "first if other and flag else second",
            "first if other is None else second",
        ] {
            for condition in ["flag", "flag is None"] {
                for (cleanup, live_successor) in [
                    ("finally:", false),
                    ("finally:", true),
                    ("except ValueError:", true),
                ] {
                    let managed = if manager.is_empty() {
                        String::new()
                    } else {
                        let work = if scope == "def" {
                            "operation()"
                        } else {
                            "await awaitable"
                        };
                        format!("            {manager} manager:\n                {work}\n")
                    };
                    let successor = if live_successor {
                        "        operation()\n"
                    } else {
                        ""
                    };
                    let source = format!(
                        "{scope} sample(flag, other, first, second, operation, manager, awaitable):\n    try:\n        if other:\n            operation()\n        else:\n            value = {value}\n{managed}            if {condition}:\n                assert value\n            else:\n                assert other\n{successor}    {cleanup}\n        operation()\n"
                    );
                    assert_matches_cpython(
                        &source,
                        &format!(
                            "completed_ternary_prefix_{}_{}_{}_{}_{}_{}.py",
                            scope.len(),
                            manager.len(),
                            value.len(),
                            condition.len(),
                            cleanup.len(),
                            live_successor
                        ),
                        &LEVELS,
                    );
                }
            }
        }
    }
}

#[test]
fn guarded_finalizer_managers_preserve_erased_arm_joins() {
    for (scope, manager) in [
        ("def", "with"),
        ("async def", "with"),
        ("async def", "async with"),
    ] {
        for work in ["operation()", "await operation()"] {
            if scope == "def" && work.starts_with("await ") {
                continue;
            }
            for context in ["manager", "operation()", "manager as value"] {
                for guard in ["other", "other and operation()"] {
                    for condition in ["flag", "flag is None"] {
                        for placement in ["true", "false", "tail"] {
                            let guarded = match placement {
                                "true" => format!(
                                    "        if {guard}:\n            {manager} {context}:\n                {work}\n        else:\n            operation()\n"
                                ),
                                "false" => format!(
                                    "        if {guard}:\n            operation()\n        else:\n            {manager} {context}:\n                {work}\n"
                                ),
                                _ => format!(
                                    "        if {guard}:\n            operation()\n        {manager} {context}:\n            {work}\n"
                                ),
                            };
                            let source = format!(
                                "{scope} sample(flag, other, operation, manager):\n    try:\n        value = operation()\n        if {condition}:\n            assert value\n        else:\n            assert other\n    finally:\n{guarded}"
                            );
                            assert_matches_cpython(
                                &source,
                                &format!(
                                    "guarded_finalizer_manager_{}_{}_{}_{}_{}_{}_{}.py",
                                    scope.len(),
                                    manager.len(),
                                    work.len(),
                                    context.len(),
                                    guard.len(),
                                    condition.len(),
                                    placement
                                ),
                                &LEVELS,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn short_circuit_finalizer_guards_preserve_erased_arm_joins() {
    for scope in ["def", "async def"] {
        for work in ["operation()", "await operation()"] {
            if scope == "def" && work.starts_with("await ") {
                continue;
            }
            for guard in [
                "other",
                "other and operation()",
                "other or operation()",
                "other is not None and not operation()",
                "not (other or operation())",
                "(other and flag) or operation()",
            ] {
                for condition in ["flag", "flag is None"] {
                    for alternate in [false, true] {
                        for nested in [false, true] {
                            let protected = if nested {
                                format!(
                                    "        if other:\n            operation()\n        else:\n            if {condition}:\n                assert value\n            else:\n                assert other\n"
                                )
                            } else {
                                format!(
                                    "        if {condition}:\n            assert value\n        else:\n            assert other\n"
                                )
                            };
                            let alternate = if alternate {
                                format!(
                                    "        elif flag and operation():\n            {work}\n        else:\n            operation()\n"
                                )
                            } else {
                                String::new()
                            };
                            let source = format!(
                                "{scope} sample(flag, other, operation):\n    try:\n        value = operation()\n{protected}    finally:\n        if {guard}:\n            {work}\n{alternate}        operation()\n"
                            );
                            assert_matches_cpython(
                                &source,
                                &format!(
                                    "short_circuit_finalizer_{}_{}_{}_{}_{}_{}.py",
                                    scope.len(),
                                    work.len(),
                                    guard.len(),
                                    condition.len(),
                                    alternate.len(),
                                    nested
                                ),
                                &LEVELS,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn completed_manager_loops_preserve_nested_erased_arm_continuations() {
    for (scope, manager) in [
        ("def", "with"),
        ("async def", "with"),
        ("async def", "async with"),
    ] {
        for work in ["operation()", "await operation()", "await awaitable"] {
            if scope == "def" && work.starts_with("await ") {
                continue;
            }
            for loop_test in ["other", "operation(other) < 1"] {
                for condition in ["flag", "flag is None"] {
                    for (cleanup, live_successor) in [
                        ("finally:", false),
                        ("finally:", true),
                        ("except ValueError:", true),
                    ] {
                        let successor = if live_successor {
                            if scope == "def" {
                                "        operation()\n"
                            } else {
                                "        await operation()\n"
                            }
                        } else {
                            ""
                        };
                        let source = format!(
                            "{scope} sample(flag, other, manager, operation, awaitable):\n    try:\n        if other:\n            operation()\n        else:\n            {manager} manager:\n                while {loop_test}:\n                    {work}\n            value = operation()\n            if {condition}:\n                assert value\n            else:\n                assert other\n{successor}    {cleanup}\n        operation()\n"
                        );
                        assert_matches_cpython(
                            &source,
                            &format!(
                                "completed_manager_loop_{}_{}_{}_{}_{}_{}_{}.py",
                                scope.len(),
                                manager.len(),
                                work.len(),
                                loop_test.len(),
                                condition.len(),
                                cleanup.len(),
                                successor.len()
                            ),
                            &LEVELS,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn coroutine_erased_boolean_arm_keeps_shared_iterator_exit() {
    for scope in ["def", "async def"] {
        for outer in ["flag", "flag in {1, 2}"] {
            for condition in ["first or second", "first or second or third"] {
                for prefix in ["", "    prepare()\n"] {
                    for body in ["assert item", "assert item\n                assert other"] {
                        let source = format!(
                            "{scope} sample(items, flag, first, second, third, other):\n{prefix}    if {outer}:\n        for item in items:\n            if {condition}:\n                {body}\n"
                        );
                        assert_matches_cpython(
                            &source,
                            &format!(
                                "erased_boolean_iterator_{}_{}_{}_{}_{}.py",
                                scope.len(),
                                outer.len(),
                                condition.len(),
                                prefix.len(),
                                body.len()
                            ),
                            &LEVELS,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn completed_managers_preserve_nested_erased_arm_continuations() {
    for (scope, manager) in [
        ("def", "with"),
        ("async def", "with"),
        ("async def", "async with"),
    ] {
        for work in ["operation()", "await operation()", "await awaitable"] {
            if scope == "def" && work.starts_with("await ") {
                continue;
            }
            for context in ["manager", "operation(manager)"] {
                for binding in ["", " as value"] {
                    for condition in ["flag", "flag is None"] {
                        for (cleanup, successor) in [
                            ("finally:", ""),
                            ("finally:", "        operation()\n"),
                            ("except ValueError:", "        operation()\n"),
                        ] {
                            let source = format!(
                                "{scope} sample(flag, other, manager, operation, awaitable):\n    try:\n        if other:\n            operation()\n        else:\n            operation()\n            {manager} {context}{binding}:\n                {work}\n            if {condition}:\n                assert other\n            else:\n                assert flag\n        assert manager\n{successor}    {cleanup}\n        operation()\n"
                            );
                            assert_matches_cpython(
                                &source,
                                &format!(
                                    "completed_manager_continuation_{}_{}_{}_{}_{}_{}_{}_{}.py",
                                    scope.len(),
                                    manager.len(),
                                    work.len(),
                                    context.len(),
                                    binding.len(),
                                    condition.len(),
                                    cleanup.len(),
                                    successor.len()
                                ),
                                &LEVELS,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn protected_erased_arms_keep_joins_before_finalizer_assignments() {
    for scope in ["def", "async def"] {
        for value in [
            "operation()",
            "operation(flag)",
            "await operation()",
            "await awaitable",
        ] {
            if scope == "def" && value.starts_with("await ") {
                continue;
            }
            for condition in ["flag", "flag is None"] {
                for guarded in [false, true] {
                    for prefix in ["", "        operation()\n"] {
                        let assignment = if guarded {
                            format!(
                                "        if other:\n            value = {value}\n        else:\n            value = {value}\n"
                            )
                        } else {
                            format!("        value = {value}\n")
                        };
                        let source = format!(
                            "{scope} sample(flag, other, operation, awaitable):\n    try:\n        operation()\n        if {condition}:\n            assert other\n        else:\n            assert flag\n    finally:\n{prefix}{assignment}        operation(value)\n"
                        );
                        assert_matches_cpython(
                            &source,
                            &format!(
                                "finalizer_assignment_entry_{}_{}_{}_{guarded}_{}.py",
                                scope.len(),
                                value.len(),
                                condition.len(),
                                prefix.len()
                            ),
                            &LEVELS,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn protected_erased_arms_keep_joins_before_finalizer_managers() {
    for (scope, manager) in [
        ("def", "with"),
        ("async def", "with"),
        ("async def", "async with"),
    ] {
        for context in ["manager", "operation(manager)"] {
            for binding in ["", " as value"] {
                for condition in ["flag", "flag is None"] {
                    for prefix in ["", "        operation()\n"] {
                        for suffix in ["", "        operation()\n"] {
                            let source = format!(
                                "{scope} sample(flag, manager, operation):\n    try:\n        operation()\n        if {condition}:\n            assert manager\n        else:\n            assert flag\n    finally:\n{prefix}        {manager} {context}{binding}:\n            operation()\n{suffix}"
                            );
                            assert_matches_cpython(
                                &source,
                                &format!(
                                    "finalizer_manager_entry_{}_{}_{}_{}_{}_{}_{}.py",
                                    scope.len(),
                                    manager.len(),
                                    context.len(),
                                    binding.len(),
                                    condition.len(),
                                    prefix.len(),
                                    suffix.len()
                                ),
                                &LEVELS,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn protected_nested_erased_arms_inherit_live_successors() {
    for scope in ["def", "async def"] {
        for protection in ["finally", "except", "none"] {
            for depth in 1..=2 {
                for prefix in ["", "value = operation()"] {
                    for condition in ["flag", "flag is None"] {
                        for successor in ["operation()", "await operation()", "await awaitable"] {
                            if scope == "def" && successor.starts_with("await ") {
                                continue;
                            }
                            let base = if protection == "none" {
                                "    "
                            } else {
                                "        "
                            };
                            let mut source =
                                format!("{scope} sample(flag, other, operation, awaitable):\n");
                            if protection != "none" {
                                source.push_str("    try:\n");
                            }
                            let mut indent = base.to_owned();
                            for _ in 0..depth {
                                writeln!(
                                    source,
                                    "{indent}if other:\n{indent}    operation()\n{indent}else:"
                                )
                                .unwrap();
                                indent.push_str("    ");
                            }
                            if !prefix.is_empty() {
                                writeln!(source, "{indent}{prefix}").unwrap();
                            }
                            writeln!(source, "{indent}if {condition}:\n{indent}    assert other\n{indent}else:\n{indent}    assert flag\n{base}{successor}").unwrap();
                            if protection != "none" {
                                let cleanup = if protection == "finally" {
                                    "finally:"
                                } else {
                                    "except ValueError:"
                                };
                                writeln!(source, "    {cleanup}\n        operation()").unwrap();
                            }
                            assert_matches_cpython(
                                &source,
                                &format!(
                                    "nested_live_successor_{}_{}_{}_{}_{}_{}.py",
                                    scope.len(),
                                    protection,
                                    depth,
                                    prefix.len(),
                                    condition.len(),
                                    successor.len()
                                ),
                                &LEVELS,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn completed_manager_arm_preserves_iterator_exit() {
    for (scope, outer) in [
        ("def", "with"),
        ("async def", "with"),
        ("async def", "async with"),
    ] {
        for condition in ["flag", "flag is None"] {
            for body in ["operation()", "await operation()"] {
                if scope == "def" && body.starts_with("await") {
                    continue;
                }
                for binding in ["", " as value"] {
                    for prefix in ["", "                    assert other\n"] {
                        let source = format!(
                            "{scope} sample(items, flag, other, context, operation):\n    {outer} context:\n        for item in items:\n            if {condition}:\n                with context{binding}:\n{prefix}                    {body}\n            else:\n                assert other\n"
                        );
                        assert_matches_cpython(&source, "completed_iterator_manager.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn completed_generator_store_preserves_finally_iterator_exit() {
    for scope in ["def", "async def"] {
        for value in [
            "sum(x * x for x in values)",
            "operation(x for x in values if x)",
            "operation.part(x if flag else other for x in values)",
            "operation(sum(x * x for x in values))",
        ] {
            for prefix in ["", "                assert other\n"] {
                for suffix in ["", "                assert flag\n"] {
                    let source = format!(
                        "{scope} sample(items, values, flag, other, operation):\n    try:\n        for item in items:\n            if flag:\n{prefix}                result = {value}\n{suffix}            else:\n                assert other\n    finally:\n        operation()\n"
                    );
                    assert_matches_cpython(&source, "generator_store_iterator.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn erased_iterator_arms_keep_enclosing_managers() {
    for manager in ["with", "async with"] {
        for managers in ["manager()", "manager(), other()"] {
            for outer in [false, true] {
                for test in ["flag", "left or right", "left == right"] {
                    for assertion in ["assert value", "assert await operation()"] {
                        let body = format!(
                            "            for value in items:\n                if {test}:\n                    {assertion}\n                else:\n                    assert await other()\n"
                        );
                        let source = if outer {
                            format!(
                                "async def sample(items, flag, left, right):\n    {manager} {managers}:\n        try:\n{body}        finally:\n            await finish()\n"
                            )
                        } else {
                            format!(
                                "async def sample(items, flag, left, right):\n    try:\n        {manager} {managers}:\n{body}    finally:\n        await finish()\n"
                            )
                        };
                        assert_matches_cpython(&source, "erased_managed_iterator.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn enclosing_manager_preserves_normalized_finally_iterator_exit() {
    for scope in ["def", "async def"] {
        for managers in [
            "manager()",
            "manager(), other()",
            "manager(), other(), third(), fourth()",
        ] {
            for test in ["flag", "left or right", "left == right"] {
                for body in ["assert value", "operation()", "value = operation()"] {
                    let source = format!(
                        "{scope} sample(items, flag, left, right):\n    with {managers}:\n        try:\n            for value in items:\n                if {test}:\n                    {body}\n                else:\n                    assert other\n        finally:\n            if flag:\n                operation()\n"
                    );
                    assert_matches_cpython(&source, "managed_finally_iterator.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn coroutine_finally_iterator_erased_else_uses_normalized_backedge() {
    for test in ["flag", "left or right", "left == right"] {
        for body in ["assert value", "operation()", "value = operation()"] {
            for cleanup in ["operation()", "await operation()"] {
                let source = format!(
                    "async def sample(items, flag, left, right):\n    try:\n        for value in items:\n            if {test}:\n                {body}\n            else:\n                assert other\n    finally:\n        {cleanup}\n"
                );
                assert_matches_cpython(&source, "finally_iterator_condition.py", &LEVELS);
            }
        }
    }
}

#[test]
fn resumable_manager_break_threads_scope_backedge() {
    for (scope, work) in [("def", "yield value"), ("async def", "await operation()")] {
        for outer in ["while True", "while flag"] {
            for managers in ["first()", "first(), second()"] {
                for handler in ["Error", "Error as error"] {
                    let source = format!(
                        "{scope} sample():\n    {outer}:\n        while True:\n            try:\n                with {managers}:\n                    {work}\n            except {handler}:\n                raise\n            else:\n                with third():\n                    operation()\n                break\n"
                    );
                    assert_matches_cpython(&source, "resumable_manager_break.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn protected_erased_arms_keep_joins_before_awaited_successors() {
    for protection in ["finally", "except", "none"] {
        for prefix in ["", "    await operation()\n"] {
            for condition in ["flag", "flag is None"] {
                for successor in [
                    "operation()",
                    "await operation()",
                    "await operation(other or flag)",
                    "await awaitable",
                ] {
                    let indent = if protection == "none" {
                        "    "
                    } else {
                        "        "
                    };
                    let mut source =
                        format!("async def sample(flag, other, operation, awaitable):\n{prefix}");
                    if protection != "none" {
                        source.push_str("    try:\n");
                    }
                    writeln!(source, "{indent}if {condition}:\n{indent}    assert other\n{indent}else:\n{indent}    assert flag\n{indent}{successor}").unwrap();
                    if protection != "none" {
                        let cleanup = if protection == "finally" {
                            "finally:"
                        } else {
                            "except ValueError:"
                        };
                        writeln!(source, "    {cleanup}\n        operation()").unwrap();
                    }
                    assert_matches_cpython(
                        &source,
                        &format!(
                            "protected_awaited_successor_{}_{}_{}_{}.py",
                            protection,
                            prefix.len(),
                            condition.len(),
                            successor.len()
                        ),
                        &LEVELS,
                    );
                }
            }
        }
    }
}

#[test]
fn protected_async_manager_exits_keep_erased_arm_joins() {
    for manager in ["with", "async with"] {
        for prefix in ["operation()", "await operation()"] {
            for condition in ["flag", "flag is None"] {
                for suffix in [
                    "",
                    "            assert manager\n",
                    "            operation()\n",
                ] {
                    for cleanup in ["operation()", "await operation()"] {
                        let source = format!(
                            "async def sample(flag, manager, operation):\n    try:\n        {manager} manager:\n            {prefix}\n            if {condition}:\n                assert manager\n            else:\n                assert flag\n{suffix}    finally:\n        {cleanup}\n"
                        );
                        assert_matches_cpython(
                            &source,
                            &format!(
                                "protected_manager_exit_{}_{}_{}_{}_{}.py",
                                manager.len(),
                                prefix.len(),
                                condition.len(),
                                suffix.len(),
                                cleanup.len()
                            ),
                            &LEVELS,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn retained_manager_preserves_handler_return_none_owner() {
    for (scope, manager) in [
        ("def", "with"),
        ("async def", "with"),
        ("async def", "async with"),
    ] {
        for (index, value) in [
            "value",
            "make(value)",
            "(value, flag)",
            "(value, make(flag))",
        ]
        .into_iter()
        .enumerate()
        {
            let source = format!(
                "{scope} sample(manager, value, flag):\n    {manager} manager:\n        try:\n            operation()\n        except Exception:\n            return {value}\n        finally:\n            if value is not None:\n                finish(value)\n"
            );
            assert_matches_cpython(
                &source,
                &format!(
                    "retained_handler_none_{}_{index}.py",
                    manager.len() + scope.len()
                ),
                &LEVELS,
            );
        }
    }
}

#[test]
fn manager_try_else_break_threads_outer_backedge() {
    for (scope, work) in [
        ("def", "operation()"),
        ("def", "yield value"),
        ("async def", "await operation()"),
    ] {
        for outer in ["while True", "while flag", "for item in items"] {
            for managers in ["first()", "first(), second()"] {
                for handler in ["Error", "Error as error"] {
                    let suffixes: &[&str] = if work == "operation()" {
                        &["", "        operation()\n"]
                    } else {
                        &["        operation()\n"]
                    };
                    for suffix in suffixes {
                        let source = format!(
                            "{scope} sample():\n    {outer}:\n        while True:\n            try:\n                with {managers}:\n                    {work}\n            except {handler}:\n                raise\n            else:\n                with third():\n                    operation()\n                break\n{suffix}"
                        );
                        assert_matches_cpython(&source, "manager_try_else_break.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn handler_manager_return_repeats_small_exit_copying() {
    for scope in ["def", "async def"] {
        for (managers, handler, guarded) in [
            ("self.manager", "Exception", false),
            ("self.manager", "Exception", true),
            ("self.manager, other", "Exception", false),
            ("self.manager, other", "Exception", true),
            ("self.manager", "Exception as error", false),
        ] {
            for returned in ["True", "None", "'done'"] {
                let body = if guarded {
                    "                    if value is None:\n                        self.total += 1\n"
                } else {
                    "                    self.total += 1\n"
                };
                let source = format!(
                    "class Sample(Base):\n    {scope} sample(self, unused):\n        try:\n            if predicate():\n                with {managers}:\n{body}        except {handler}:\n            with {managers}:\n                self.total += 1\n        return {returned}\n"
                );
                assert_matches_cpython(&source, "handler_manager_return.py", &LEVELS);
            }
        }
    }
}

#[test]
fn nested_finalizers_preserve_manager_suppression_return_copy() {
    for (scope, operation) in [
        ("def", "operation()"),
        ("def", "yield value"),
        ("async def", "await operation()"),
    ] {
        for outer in ["assert flag", "finish()", "return value"] {
            for inner in ["assert flag", "finish()", "pass"] {
                let source = format!(
                    "{scope} sample(manager, value, flag):\n    try:\n        try:\n            with manager:\n                {operation}\n        finally:\n            {inner}\n    finally:\n        {outer}\n"
                );
                assert_matches_cpython_versions(
                    &source,
                    "nested_finally_manager.py",
                    &LEVELS,
                    &[PythonVersion::Py314, PythonVersion::Py315],
                );
            }
        }
    }
}

#[test]
fn protected_erased_arms_keep_joins_before_iterator_successors() {
    for scope in ["def", "async def"] {
        for protection in ["finally", "except", "loop"] {
            for iterator in ["items", "operation(items)"] {
                for condition in ["flag", "flag is None"] {
                    for body in ["assert other", "operation(other)"] {
                        for suffix in ["", "operation()\n"] {
                            let indent = if protection == "loop" {
                                "            "
                            } else {
                                "        "
                            };
                            let mut source =
                                format!("{scope} sample(flag, items, operation):\n    try:\n");
                            if protection == "loop" {
                                source.push_str("        for item in items:\n");
                            }
                            writeln!(source,
                                "{indent}if {condition}:\n{indent}    assert items\n{indent}else:\n{indent}    assert flag\n{indent}for other in {iterator}:\n{indent}    {body}"
                            ).unwrap();
                            if !suffix.is_empty() {
                                write!(source, "{indent}{suffix}").unwrap();
                            }
                            let cleanup = if protection == "except" {
                                "except ValueError:"
                            } else {
                                "finally:"
                            };
                            writeln!(source, "    {cleanup}\n        operation()").unwrap();
                            assert_matches_cpython(
                                &source,
                                &format!(
                                    "protected_iterator_{}_{}_{}_{}_{}_{}.py",
                                    scope.len(),
                                    protection,
                                    iterator.len(),
                                    condition.len(),
                                    body.len(),
                                    suffix.len()
                                ),
                                &LEVELS,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn erased_normal_finally_arms_keep_conditional_returns() {
    for (mode, (scope, entry)) in [
        ("def", ""),
        ("async def", ""),
        ("async def", "    await operation()\n"),
        ("def", "    yield operation()\n"),
        ("async def", "    yield operation()\n"),
    ]
    .into_iter()
    .enumerate()
    {
        for prefix in [
            "",
            "        operation()\n",
            "        try:\n            operation()\n        finally:\n            operation()\n",
        ] {
            for condition in ["flag", "flag is None", "operation()"] {
                for body in ["assert flag", "pass"] {
                    for suffix in ["", "        assert value\n"] {
                        let source = format!(
                            "{scope} sample(flag, operation):\n{entry}    try:\n        operation()\n    finally:\n{prefix}        value = operation()\n        assert value\n        if {condition}:\n            {body}\n{suffix}"
                        );
                        assert_matches_cpython(
                            &source,
                            &format!(
                                "erased_finally_arm_{mode}_{}_{}_{}_{}.py",
                                prefix.len(),
                                condition.len(),
                                body.len(),
                                suffix.len()
                            ),
                            &LEVELS,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn erased_coroutine_elif_chains_keep_each_return_donor() {
    for prefix in ["", "    await operation()\n"] {
        for compound in [false, true] {
            for clauses in 1..=3 {
                for suffix in ["", "        assert value\n"] {
                    for otherwise in ["", "    else:\n        operation()\n"] {
                        let mut source = format!(
                            "async def sample(flag, other, value, operation):\n{prefix}    if flag:\n        value = operation()\n        assert value\n"
                        );
                        for index in 0..=clauses {
                            let keyword = if index == 0 { "if" } else { "elif" };
                            let condition = if compound && index < clauses {
                                format!("other and value == {index}")
                            } else {
                                "other".to_owned()
                            };
                            writeln!(
                                source,
                                "        {keyword} {condition}:\n            assert value"
                            )
                            .unwrap();
                        }
                        source.push_str(suffix);
                        source.push_str(otherwise);
                        assert_matches_cpython(
                            &source,
                            &format!(
                                "erased_coroutine_elifs_{}_{}_{}_{}_{}.py",
                                prefix.len(),
                                compound,
                                clauses,
                                suffix.len(),
                                otherwise.len()
                            ),
                            &LEVELS,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn completed_coroutine_calls_keep_false_returns_before_erased_assertions() {
    for prefix in ["", "    await operation()\n"] {
        for awaited in ["", "await "] {
            for argument in [
                "",
                "other or ''",
                "left if other else right",
                "dict(value=other or '')",
            ] {
                for condition in ["other", "other == 'ready'"] {
                    for suffix in ["", "        assert other\n"] {
                        for first in ["        operation()\n", "        assert flag\n"] {
                            for swap in [false, true] {
                                let completed = format!(
                                    "        if {condition}:\n            {awaited}operation({argument})\n{suffix}"
                                );
                                let (first, second) = if swap {
                                    (completed.as_str(), first)
                                } else {
                                    (first, completed.as_str())
                                };
                                let source = format!(
                                    "async def sample(flag, other, left, right, operation):\n{prefix}    if flag:\n{first}    else:\n{second}"
                                );
                                assert_matches_cpython(
                                    &source,
                                    &format!(
                                        "completed_coroutine_calls_{}_{}_{}_{}_{}_{}_{}.py",
                                        prefix.len(),
                                        awaited.len(),
                                        argument.len(),
                                        condition.len(),
                                        suffix.len(),
                                        first.len(),
                                        swap
                                    ),
                                    &LEVELS,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn erased_nested_boolean_arms_preserve_resumable_footer_order() {
    for (mode, (scope, prefix)) in [
        ("def", ""),
        ("async def", ""),
        ("async def", "    await operation()\n"),
        ("def", "    yield operation()\n"),
        ("async def", "    yield operation()\n"),
    ]
    .into_iter()
    .enumerate()
    {
        for (condition_index, condition) in [
            "outer is not None and (not flag or left != right)",
            "outer and (flag or left)",
            "outer and (flag or (left and right))",
            "outer and ((flag and left) or right)",
            "outer and (flag or left) and right",
            "(outer and flag) or (left and right)",
            "outer and (flag and left != right)",
            "outer and not (flag and left != right)",
        ]
        .into_iter()
        .enumerate()
        {
            for (body_index, body) in ["assert outer", "pass", "operation()"]
                .into_iter()
                .enumerate()
            {
                let source = format!(
                    "{scope} sample(outer, flag, left, right, operation):\n{prefix}    if {condition}:\n        {body}\n"
                );
                assert_matches_cpython(
                    &source,
                    &format!("nested_boolean_footer_{mode}_{condition_index}_{body_index}.py"),
                    &LEVELS,
                );
            }
        }
    }
}

#[test]
fn normal_finally_loop_exit_preserves_manager_owner() {
    for scope in ["def", "async def"] {
        for managers in ["manager", "manager as first, other as second"] {
            for condition in ["flag", "check()", "all(item.ready() for item in items)"] {
                for surrounding in [false, true] {
                    let body = if surrounding {
                        "            operation()\n"
                    } else {
                        "            try:\n                operation()\n            except Error:\n                result = check() is None\n"
                    };
                    for suffix in ["", "            operation()\n"] {
                        let source = format!(
                            "{scope} sample(flag, items):\n    with {managers}:\n        try:\n{body}        finally:\n            while True:\n                if {condition}:\n                    break\n{suffix}"
                        );
                        if condition == "check()" {
                            assert_matches_cpython(
                                &source,
                                "normal_finally_loop_owner.py",
                                &LEVELS,
                            );
                        } else {
                            assert_matches_cpython_versions(
                                &source,
                                "normal_finally_loop_owner.py",
                                &LEVELS,
                                &[PythonVersion::Py315],
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn conditional_finally_awaits_preserve_erased_assertion_exits() {
    for prefix in [
        "",
        "    await operation()\n",
        "    def capture():\n        return flag, items, operation\n",
    ] {
        for protected in [
            "        await operation()\n",
            "        if other:\n            await operation()\n",
        ] {
            for condition in ["flag", "flag is None"] {
                for body in [
                    "            await operation()\n",
                    "            for item in items:\n                await operation(item)\n",
                    "            await operation()\n            for item in items:\n                await operation(item)\n",
                ] {
                    for suffix in ["", "        assert flag\n"] {
                        for otherwise in [
                            "",
                            "        else:\n            for item in items:\n                await operation(item)\n",
                        ] {
                            let source = format!(
                                "async def sample(flag, other, items, operation):\n{prefix}    try:\n{protected}    finally:\n        if {condition}:\n{body}{otherwise}{suffix}"
                            );
                            assert_matches_cpython(
                                &source,
                                &format!(
                                    "conditional_finally_awaits_{}_{}_{}_{}_{}_{}.py",
                                    prefix.len(),
                                    protected.len(),
                                    condition.len(),
                                    body.len(),
                                    suffix.len(),
                                    otherwise.len()
                                ),
                                &LEVELS,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn protected_forward_join_keeps_async_manager_continuation() {
    for (scope, work) in [
        ("def", "operation()"),
        ("async def", "operation()"),
        ("async def", "await operation()"),
    ] {
        for continuation in ["call", "with", "async with"] {
            if scope == "def" && continuation == "async with" {
                continue;
            }
            for prefix in ["", "            if flag:\n                assert False\n"] {
                for extra_elif in [
                    "",
                    "            elif other == 1:\n                assert False\n",
                ] {
                    for live_else in [false, true] {
                        for suffix in ["", "        operation()\n"] {
                            let otherwise = if live_else {
                                format!(
                                    "            else:\n                with manager:\n                    {work}\n"
                                )
                            } else {
                                String::new()
                            };
                            let following = if continuation == "call" {
                                format!("            {work}\n")
                            } else {
                                format!(
                                    "            {continuation} manager:\n                if flag:\n                    assert False\n"
                                )
                            };
                            let source = format!(
                                "{scope} sample(manager, operation, flag, other):\n    with manager:\n        try:\n{prefix}            if other is None:\n                assert False\n            elif flag:\n                assert False\n{extra_elif}{otherwise}{following}        finally:\n            {work}\n{suffix}"
                            );
                            let filename = format!(
                                "forward_continuation_{scope}_{continuation}_{live_else}_{}_{}_{}.py",
                                prefix.is_empty(),
                                extra_elif.is_empty(),
                                suffix.is_empty()
                            );
                            assert_matches_cpython(&source, &filename, &LEVELS);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn protected_forward_join_keeps_repeated_predecessors() {
    for (scope, work) in [
        ("def", "operation()"),
        ("async def", "operation()"),
        ("async def", "await operation()"),
    ] {
        for manager in ["with", "async with"] {
            if scope == "def" && manager == "async with" {
                continue;
            }
            for prefix in ["", "            if flag:\n                assert False\n"] {
                for condition in ["other", "other is None", "other == 1"] {
                    for managed in [false, true] {
                        for suffix in ["", "        operation()\n"] {
                            let body = if managed {
                                format!(
                                    "                with manager:\n                    {work}\n"
                                )
                            } else {
                                format!("                {work}\n")
                            };
                            let source = format!(
                                "{scope} sample(manager, operation, flag, other):\n    {manager} manager:\n        try:\n{prefix}            if {condition}:\n                assert False\n            else:\n{body}        finally:\n            {work}\n{suffix}"
                            );
                            let filename = format!(
                                "protected_forward_{scope}_{manager}_{condition}_{managed}_{}_{}.py",
                                prefix.is_empty(),
                                suffix.is_empty()
                            );
                            assert_matches_cpython(&source, &filename, &LEVELS);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn nested_comparisons_preserve_erased_branch_scope_footer() {
    for scope in ["def", "async def"] {
        for prefix in ["", "    if flag:\n        assert False\n"] {
            for outer in ["other", "other == 1", "other != 1"] {
                for inner in [
                    "value in items",
                    "value not in items",
                    "value == other",
                    "value < other",
                    "value.part in items.parts",
                ] {
                    for erased_prefix in ["", "        assert other\n"] {
                        for suffix in ["", "    operation()\n"] {
                            let source = format!(
                                "{scope} sample(flag, other, value, items, operation):\n{prefix}    if {outer}:\n{erased_prefix}        if {inner}:\n            assert False\n        else:\n            assert False\n{suffix}"
                            );
                            let filename = format!(
                                "nested_comparison_{scope}_{outer}_{inner}_{}_{}_{}.py",
                                prefix.is_empty(),
                                erased_prefix.is_empty(),
                                suffix.is_empty()
                            );
                            assert_matches_cpython(&source, &filename, &LEVELS);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn coroutine_assignment_before_erased_else_keeps_original_scope_footer() {
    for prefix in [
        "",
        "    if flag:\n        assert False\n",
        "    if flag:\n        await operation()\n",
    ] {
        for condition in ["other", "not other", "other == 1"] {
            for arm_prefix in [
                "",
                "        value = await operation()\n",
                "        value = operation() if flag else other\n",
            ] {
                for value in [
                    "operation()",
                    "await operation()",
                    "operation() if flag else other",
                ] {
                    for tail in ["", "        assert value\n"] {
                        for suffix in ["", "    operation()\n"] {
                            let source = format!(
                                "async def sample(flag, other, operation):\n{prefix}    if {condition}:\n{arm_prefix}        value = {value}\n{tail}    else:\n        assert False\n{suffix}"
                            );
                            assert_matches_cpython(
                                &source,
                                "coroutine_assignment_footer.py",
                                &LEVELS,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn synchronous_manager_guards_keep_original_scope_footer() {
    for (scope, work) in [
        ("def", "operation()"),
        ("async def", "operation()"),
        ("async def", "await operation()"),
    ] {
        for prefix in [
            "",
            "    if flag:\n        assert False\n",
            "    if flag:\n        operation()\n",
        ] {
            for condition in ["other", "not other"] {
                for managers in ["manager", "make_manager()", "manager, other_manager"] {
                    for body in [
                        format!("            {work}\n"),
                        format!("            if flag:\n                {work}\n"),
                    ] {
                        for suffix in ["", "    operation()\n"] {
                            let source = format!(
                                "{scope} sample(flag, other, manager, other_manager, operation):\n{prefix}    if {condition}:\n        with {managers}:\n{body}{suffix}"
                            );
                            assert_matches_cpython(
                                &source,
                                "synchronous_manager_footer.py",
                                &LEVELS,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn iterator_guard_prefixes_preserve_footer_before_erased_assertion() {
    for prefix in [
        "        assert False\n",
        "        values = [item for item in items if item]\n        assert False\n",
        "        if flag:\n            operation()\n",
        "        with manager:\n            operation()\n",
        "        try:\n            operation()\n        finally:\n            operation()\n",
    ] {
        for startup in [
            "",
            "    if flag:\n        assert False\n",
            "    if flag:\n        operation()\n    else:\n        if other:\n            assert False\n",
        ] {
            for body in [
                "            if flag:\n                assert False\n",
                "            if flag:\n                assert False\n            else:\n                assert False\n",
                "            operation(item)\n",
            ] {
                for suffix in ["    assert False\n", "    assert False\n    operation()\n"] {
                    let source = format!(
                        "def sample(flag, other, items, operation, manager):\n{startup}    if other:\n{prefix}        for item in items:\n{body}{suffix}"
                    );
                    assert_matches_cpython(&source, "iterator_guard_prefix_footer.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn erased_assertion_after_iterator_guard_keeps_scope_footer() {
    for prefix in [
        "",
        "    if flag:\n        assert False\n",
        "    operation()\n",
    ] {
        for iterator in ["items", "obj.items"] {
            for body in [
                "            assert False\n",
                "            if flag:\n                assert False\n",
                "            operation(item)\n",
            ] {
                for suffix in [
                    "",
                    "    assert False\n",
                    "    assert False\n    operation()\n",
                ] {
                    let source = format!(
                        "def sample(flag, other, items, obj, operation):\n{prefix}    if other:\n        for item in {iterator}:\n{body}{suffix}"
                    );
                    assert_matches_cpython(&source, "erased_iterator_guard_suffix.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn erased_assertion_after_iterator_guard_preserves_loop_exits() {
    for prefix in ["", "    if flag:\n        assert False\n"] {
        for body in [
            "            if flag:\n                break\n",
            "            if flag:\n                continue\n",
            "            if flag:\n                return item\n",
            "            operation(item)\n        else:\n            operation()\n",
            "            try:\n                operation(item)\n            finally:\n                operation()\n",
            "            with manager:\n                operation(item)\n",
        ] {
            for suffix in [
                "",
                "    assert False\n",
                "    assert False\n    operation()\n",
            ] {
                let source = format!(
                    "def sample(flag, other, items, operation, manager):\n{prefix}    if other:\n        for item in items:\n{body}{suffix}"
                );
                assert_matches_cpython(&source, "erased_iterator_guard_exits.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_branch_preserves_nonadjacent_cleanup_jump_location() {
    for (scope, startup, cleanup) in [
        ("def", "", "cleanup()"),
        ("async def", "", "cleanup()"),
        ("async def", "", "await cleanup()"),
        ("def", "    yield 0\n", "cleanup()"),
        ("async def", "    yield 0\n", "cleanup()"),
    ] {
        for condition in ["flag", "flag is not None"] {
            for prefix in ["assert False", "operation()"] {
                for body in ["assert False", "pass"] {
                    for alternative in ["operation()", "operation.method()"] {
                        let source = format!(
                            "{scope} sample(flag, other, operation, cleanup):\n{startup}    try:\n        if {condition}:\n            {prefix}\n        if other:\n            {body}\n        else:\n            {alternative}\n    finally:\n        {cleanup}\n"
                        );
                        assert_matches_cpython(&source, "nonadjacent_cleanup_jump.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn erased_nested_call_tail_preserves_shared_scope_footer() {
    for scope in ["def", "async def"] {
        for condition in ["flag", "flag == 1"] {
            for prefix in ["assert False", "operation()"] {
                for work in ["operation()", "operation.method()"] {
                    for tail in ["assert False", "pass"] {
                        for suffix in ["", "    operation()\n"] {
                            let source = format!(
                                "{scope} sample(flag, other, operation):\n    if {condition}:\n        {prefix}\n    if other:\n        if {condition}:\n            {work}\n        {tail}\n{suffix}"
                            );
                            assert_matches_cpython(&source, "erased_nested_call_tail.py", &LEVELS);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn paired_coroutine_manager_returns_keep_locations_before_erased_assertions() {
    for (mode, (scope, manager_await, other_await)) in [
        ("def", "", ""),
        ("async def", "", ""),
        ("async def", "await ", ""),
        ("async def", "", "await "),
        ("async def", "await ", "await "),
    ]
    .into_iter()
    .enumerate()
    {
        for condition in ["inner", "inner is None"] {
            for target in ["", " as resource"] {
                for swap in [false, true] {
                    for suffix in ["", "        assert outer\n"] {
                        for argument in ["", "[operation(item) for item in items]"] {
                            let prefix = if argument.is_empty() {
                                String::new()
                            } else {
                                format!(
                                    "        previous = {other_await}operation()\n        assert previous\n"
                                )
                            };
                            let managed = format!(
                                "            with manager(){target}:\n                {manager_await}operation()\n"
                            );
                            let other = format!(
                                "            value = {other_await}operation({argument})\n            assert value\n"
                            );
                            let (first, second) = if swap {
                                (&other, &managed)
                            } else {
                                (&managed, &other)
                            };
                            let source = format!(
                                "{scope} sample(outer, inner, manager, operation, items):\n    if outer:\n{prefix}        if {condition}:\n{first}        else:\n{second}{suffix}"
                            );
                            assert_matches_cpython(
                                &source,
                                &format!(
                                    "paired_manager_{mode}_{swap}_{}_{}_{}_{}.py",
                                    condition == "inner",
                                    target.is_empty(),
                                    suffix.is_empty(),
                                    argument.is_empty()
                                ),
                                &LEVELS,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn erased_finally_allows_manager_suppression_return_copy() {
    for (scope, operation) in [
        ("def", "operation()"),
        ("def", "yield"),
        ("async def", "await operation()"),
        ("async def", "yield"),
    ] {
        for managers in ["manager", "manager as first, other as second"] {
            for cleanup in ["assert flag", "pass", "if False:\n            operation()"] {
                let source = format!(
                    "{scope} sample():\n    try:\n        with {managers}:\n            {operation}\n    finally:\n        {cleanup}\n"
                );
                assert_matches_cpython_versions(
                    &source,
                    "erased_finally_manager.py",
                    &LEVELS,
                    &[PythonVersion::Py314, PythonVersion::Py315],
                );
            }
        }
    }
}

#[test]
fn protected_conditional_store_raise_repeats_small_exit_copying() {
    for (scope, entry) in [
        ("def", "operation()"),
        ("def", "yield"),
        ("async def", "await operation()"),
    ] {
        for binding in ["", " as error"] {
            for value in ["1", "value", "operation()"] {
                let source = format!(
                    "{scope} sample(value):\n    try:\n        {entry}\n    except Error{binding}:\n        result = {value} if 0 <= value < 10 else 2\n        raise\n"
                );
                assert_matches_cpython(&source, "protected_conditional_store_raise.py", &LEVELS);
            }
        }
    }
}

#[test]
fn paired_finally_returns_keep_locations_before_erased_assertions() {
    for (scope, prefix) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield 0\n"),
        ("async def", "    yield 0\n"),
    ] {
        for condition in ["flag", "flag is None"] {
            for depth in 1..=3 {
                for suffix in ["", "assert flag\n"] {
                    let mut body = format!(
                        "if {condition}:\n    operation.first()\nelse:\n    operation.other_branch()\n{suffix}"
                    );
                    for _ in 0..depth {
                        let mut protected = String::from("try:\n    operation()\nfinally:\n");
                        for line in body.lines() {
                            writeln!(protected, "    {line}").unwrap();
                        }
                        body = protected;
                    }
                    let mut source = format!("{scope} sample(flag, operation):\n{prefix}");
                    for line in body.lines() {
                        writeln!(source, "    {line}").unwrap();
                    }
                    assert_matches_cpython(&source, "paired_finally_return_locations.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn paired_coroutine_returns_keep_locations_before_erased_assertions() {
    for scope in ["def", "async def"] {
        for condition in ["inner", "inner is None", "inner or outer"] {
            for prefix in ["", "        value = operation()\n        assert value\n"] {
                for first in ["", "            assert inner\n"] {
                    for second in ["", "            assert outer\n"] {
                        for suffix in ["", "        assert outer\n"] {
                            let source = format!(
                                "{scope} sample(outer, inner, operation):\n    if outer:\n        operation()\n    else:\n{prefix}        if {condition}:\n            operation.first()\n{first}        else:\n            operation.other_branch()\n{second}{suffix}"
                            );
                            assert_matches_cpython_versions(
                                &source,
                                "paired_coroutine_return_locations.py",
                                &LEVELS,
                                &[PythonVersion::Py313],
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn erased_assertions_inside_nested_coroutine_branches_preserve_exits() {
    for scope in ["def", "async def"] {
        let awaited = if scope == "async def" { "await " } else { "" };
        for prefix in [
            String::new(),
            format!("        result = {awaited}operation()\n"),
        ] {
            for condition in ["other", "other and flag", "other or flag"] {
                for body in [
                    "            value = operation()\n            assert other\n",
                    "            value = operation()\n            assert other\n            value = operation.value\n",
                    "            value = list(item for item in items)\n            assert other\n            value = operation.value\n            assert flag\n",
                ] {
                    let source = format!(
                        "{scope} sample(flag, other, operation, items):\n    if flag:\n        operation()\n    else:\n{prefix}        if {condition}:\n{body}        assert flag\n"
                    );
                    assert_matches_cpython_versions(
                        &source,
                        "nested_coroutine_erased_assertions.py",
                        &LEVELS,
                        &[PythonVersion::Py313],
                    );
                }
            }
        }
    }
}

#[test]
fn copied_bare_return_preserves_finalizer_none_owner() {
    for managers in ["manager", "manager, other"] {
        for loop_body in [false, true] {
            let (loop_header, indent) = if loop_body {
                ("            while True:\n", "    ")
            } else {
                ("", "")
            };
            for leading_call in [false, true] {
                let prefix = if leading_call {
                    format!("                {indent}operation()\n")
                } else {
                    String::new()
                };
                for (condition_index, condition) in [
                    "obj is not None",
                    "obj is not None and obj.ready() is None",
                    "obj is None or obj.ready() is not None",
                    "obj.value",
                    "not obj.value",
                ]
                .into_iter()
                .enumerate()
                {
                    let source = format!(
                        "def sample(manager, other, obj, flag):\n    with {managers}:\n        try:\n{loop_header}            {indent}if flag:\n{prefix}                {indent}return\n            {indent}wait()\n        except BaseException:\n            raise\n        finally:\n            if {condition}:\n                notify()\n"
                    );
                    let filename = format!(
                        "copied_bare_finalizer_none_{}_{loop_body}_{leading_call}_{condition_index}.py",
                        managers.len()
                    );
                    assert_matches_cpython_versions(
                        &source,
                        &filename,
                        &LEVELS,
                        &[PythonVersion::Py314, PythonVersion::Py315],
                    );
                    if !loop_body && condition_index != 1 {
                        let call_return = source.replace("return\n", "return produce(obj)\n");
                        assert_matches_cpython_versions(
                            &call_return,
                            &format!("call_{filename}"),
                            &LEVELS,
                            &[PythonVersion::Py314, PythonVersion::Py315],
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn terminal_with_condition_after_recovery_preserves_fresh_owner() {
    for condition in [
        "obj.value",
        "flag",
        "predicate()",
        "obj.value is None",
        "flag and obj.value",
    ] {
        for body in ["pass", "operation()"] {
            for cleanup in ["pass", "finish()"] {
                for handler in [
                    "",
                    "    except (ValueError, TypeError) as error:\n        pass\n",
                ] {
                    let source = format!(
                        "def sample(manager, obj, flag):\n    try:\n        with manager:\n            try:\n                {body}\n            except BaseException as error:\n                raise\n            if {condition}:\n                raise RuntimeError\n{handler}    finally:\n        {cleanup}\n"
                    );
                    assert_matches_cpython_versions(
                        &source,
                        "terminal_with_recovery.py",
                        &LEVELS,
                        &[PythonVersion::Py314, PythonVersion::Py315],
                    );
                }
            }
        }
    }
}

#[test]
fn normal_finally_receiver_none_preserves_async_manager_owner() {
    for asynchronous in [false, true] {
        let (header, manager, awaited) = if asynchronous {
            ("async def", "async with", "await ")
        } else {
            ("def", "with", "")
        };
        for receiver in ["obj.value", "obj.value()"] {
            for comparison in ["is", "is not"] {
                for captured in [false, true] {
                    let closure = if captured {
                        "    def retain():\n        return obj\n"
                    } else {
                        ""
                    };
                    let source = format!(
                        "{header} sample(manager, obj):\n{closure}    {manager} manager:\n        try:\n            if {awaited}obj.run():\n                raise RuntimeError\n        finally:\n            if {receiver} {comparison} None:\n                {awaited}obj.wait()\n"
                    );
                    assert_matches_cpython_versions(
                        &source,
                        "normal_finally_receiver_none.py",
                        &LEVELS,
                        &[PythonVersion::Py314, PythonVersion::Py315],
                    );
                }
            }
        }
    }
}

#[test]
fn protected_conditional_await_keeps_exits_before_erased_assertions() {
    for prefix in ["", "    await operation()\n"] {
        for condition in [
            "flag",
            "flag and other",
            "flag or other",
            "flag and not (other and third)",
        ] {
            for assigned in ["", "value = "] {
                for suffix in ["", "        assert flag\n"] {
                    let source = format!(
                        "async def sample(flag, other, third, operation):\n{prefix}    try:\n        if {condition}:\n            {assigned}await operation()\n{suffix}    except RuntimeError:\n        raise\n"
                    );
                    assert_matches_cpython_versions(
                        &source,
                        "protected_conditional_await.py",
                        &LEVELS,
                        &[PythonVersion::Py313],
                    );
                }
            }
        }
    }
}

#[test]
fn nested_coroutine_guards_preserve_erased_tail_footer() {
    for scope in ["def", "async def"] {
        for outer in [
            "flag",
            "flag and other",
            "flag or other",
            "flag is None and other == 1",
        ] {
            for inner in ["if other", "if other == 1"] {
                for work in ["value = 1", "operation()"] {
                    for tail in ["assert False", "pass"] {
                        for suffix in ["", "    operation()\n"] {
                            let source = format!(
                                "{scope} sample(flag, other, operation):\n    if {outer}:\n        {inner}:\n            {work}\n        {tail}\n{suffix}"
                            );
                            assert_matches_cpython(&source, "nested_coroutine_footer.py", &LEVELS);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn nested_coroutine_loops_preserve_erased_tail_footer_312() {
    for scope in ["def", "async def"] {
        for outer in ["flag", "flag and other"] {
            for work in ["value = 1", "operation()"] {
                for tail in ["assert False", "pass"] {
                    for suffix in ["", "    operation()\n"] {
                        let source = format!(
                            "{scope} sample(flag, other, operation):\n    if {outer}:\n        while other:\n            {work}\n        {tail}\n{suffix}"
                        );
                        assert_matches_cpython_versions(
                            &source,
                            "nested_coroutine_loop_footer.py",
                            &LEVELS,
                            &[PythonVersion::Py312],
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn pruned_annotation_constants_preserve_repeated_return_witnesses() {
    for scope in ["def", "async def"] {
        for future in ["", "from __future__ import annotations\n"] {
            for docstring in ["", "    \"\"\"Summary.\"\"\"\n"] {
                for annotation in ["", " -> object"] {
                    let source = format!(
                        "{future}{scope} sample(flag, other):\n{docstring}    def nested(){annotation}:\n        pass\n    if flag:\n        if other:\n            assert False\n    elif other:\n        assert False\n    if flag:\n        assert False\n    else:\n        assert False\n"
                    );
                    assert_matches_cpython(&source, "pruned_return_witness.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn nested_call_guards_preserve_shared_scope_footer() {
    for scope in ["def", "async def"] {
        for prefix in ["", "    with manager:\n        operation()\n"] {
            for (condition, inner) in [("flag", "other"), ("flag is None", "other == 'value'")] {
                for erased in ["assert False", "operation()"] {
                    for suffix in ["", "    operation()\n"] {
                        let source = format!(
                            "{scope} sample(manager, flag, other, operation):\n{prefix}    if {condition}:\n        operation()\n    else:\n        if other:\n            {erased}\n        if {inner}:\n            operation()\n{suffix}"
                        );
                        assert_matches_cpython(&source, "nested_call_footer.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn local_annotated_conditional_stores_preserve_branch_exit_copying() {
    for scope in ["def", "async def"] {
        for annotation in ["", ": object"] {
            for suffix in ["", "        assert value\n"] {
                let source = format!(
                    "{scope} sample(inner, flag):\n    if inner:\n        value{annotation} = 1 if flag else 2\n{suffix}    else:\n        assert flag\n"
                );
                assert_matches_cpython(&source, "annotated_conditional_store.py", &LEVELS);
            }
        }
    }
    let source = "async def sample(outer, inner, flag, operation):\n    await operation()\n    if outer is None:\n        assert flag\n    else:\n        if inner:\n            value: object = (\n                1 if flag else 2\n            )\n            assert value\n        else:\n            assert flag\n";
    assert_matches_cpython(source, "nested_annotated_conditional_store.py", &LEVELS);
}

#[test]
fn iterator_try_return_counts_original_pop_block() {
    for scope in ["def", "async def"] {
        for value in [
            "0 <= operation(value) <= 10",
            "first if flag else second",
            "first or second",
            "value",
            "False",
        ] {
            let source = format!(
                "{scope} sample(items, operation, flag, first, second):\n\
                 \x20   for value in items:\n\
                 \x20       try:\n\
                 \x20           return {value}\n\
                 \x20       except ValueError:\n\
                 \x20           return False\n\
                 \x20   return False\n"
            );
            assert_matches_cpython(&source, "iterator_try_return.py", &LEVELS);
        }
    }
}

#[test]
fn normal_finally_conditional_entry_preserves_nested_setup_marker() {
    for (name, body) in [
        ("assert", "assert condition"),
        ("assert_not", "assert not condition"),
        ("assert_message", "assert condition, 'missing'"),
        (
            "assert_multiple",
            "assert condition\n            assert other",
        ),
        (
            "raise",
            "if not condition:\n                raise RuntimeError",
        ),
        ("call", "operation()"),
    ] {
        let source = format!(
            "def sample(manager, obj, condition, other):\n    with manager:\n        try:\n            {body}\n        finally:\n            try:\n                if obj.value:\n                    operation()\n            finally:\n                if obj.value:\n                    operation()\n"
        );
        assert_matches_cpython_versions(
            &source,
            &format!("normal_finally_conditional_{name}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn normal_finally_method_none_owner_preserves_cell_receiver() {
    let source = "def sample():\n    with manager:\n        try:\n            pass\n        except RuntimeError:\n            pass\n        obj = make([1, 2, a(), 3, b(), c(), d(), e(), f()], x=g, y=h)\n        def retain():\n            return obj\n        try:\n            result = obj.wait()\n        finally:\n            if obj.poll() is None:\n                obj.stop()\n        finish(result)\n";
    assert_matches_cpython_versions(
        source,
        "normal_finally_cell_method_none.py",
        &LEVELS,
        &[PythonVersion::Py314, PythonVersion::Py315],
    );
}

#[test]
fn erased_branch_chain_suffix_preserves_loop_tail_polarity() {
    assert_matches_cpython(
        include_str!("fixtures/erased_branch_chain_loop_tail.py"),
        "erased_branch_chain_loop_tail.py",
        &LEVELS,
    );
}

#[test]
fn erased_module_comprehension_cells_precede_synthetic_annotations() {
    for future in ["", "from __future__ import annotations\n"] {
        for annotation in ["value: int\n", "if flag:\n    value: int\n"] {
            for body in [
                "assert [(lambda: item) for item in items]\n",
                "assert [(lambda: z) for z in items]\nassert [(lambda: a) for a in items]\n",
                "out = [(lambda: z) for z in items]\nassert [(lambda: a) for a in items]\n",
                "assert [(lambda: z) for z in items]\nout = [(lambda: a) for a in items]\n",
            ] {
                assert_matches_cpython(
                    &format!("{future}{annotation}{body}"),
                    "erased_module_cells.py",
                    &LEVELS,
                );
            }
        }
    }
}

#[test]
fn copied_finally_scope_footers_keep_repeated_predecessor_counts() {
    let source = include_str!("fixtures/copied_finally_scope_footer.py");
    for source in [
        source.to_owned(),
        source.replace("assert other", "action()"),
        source.replace("assert flag", "action()"),
    ] {
        assert_matches_cpython(&source, "copied_finally_scope_footer.py", &LEVELS);
    }
}

#[test]
fn erased_assertions_preserve_normal_finally_multiple_call_exits() {
    for (scope, prefix) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield 0\n"),
        ("async def", "    yield 0\n"),
    ] {
        for condition in ["predicate()", "flag and other"] {
            for count in 1..=4 {
                for separator in ["\n            ", "; "] {
                    let calls = (0..count)
                        .map(|index| format!("consume({index})"))
                        .collect::<Vec<_>>()
                        .join(separator);
                    let source = format!(
                        "{scope} f(flag, other):\n{prefix}    try:\n        consume()\n    finally:\n        if {condition}:\n            {calls}\n        assert other\n"
                    );
                    assert_matches_cpython(&source, "normal_finally_calls.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn conjunctions_preserve_original_ordinary_await_footer() {
    for prefix in [
        "",
        "    if other:\n        assert other\n",
        "    if other:\n        consume(other)\n",
        "    with manager:\n        consume(other)\n    if other:\n        assert other\n",
    ] {
        for leaf in [
            "        await consume(flag)\n",
            "        value = flag\n        await consume(value)\n",
            "        await consume(other)\n        await consume(flag)\n",
            "        consume(other)\n        await consume(flag)\n",
        ] {
            for condition in [
                "flag and other",
                "flag and other and third",
                "flag is None and other",
                "flag and other is None",
            ] {
                let source = format!(
                    "async def f(flag, other, third, manager):\n{prefix}    if {condition}:\n{leaf}        assert third\n    assert third\n"
                );
                assert_matches_cpython(&source, "conjunction_await_footer.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_guard_preserves_managed_await_footer() {
    for prefix in ["", "    if flag:\n        assert False\n"] {
        for condition in ["flag", "not flag", "flag and other"] {
            for manager in ["manager", "manager()", "manager as value"] {
                for suffix in ["", "    assert False\n"] {
                    let source = format!(
                        "async def sample(flag, other, manager, operation):\n{prefix}    if {condition}:\n        with {manager}:\n            await operation()\n{suffix}"
                    );
                    assert_matches_cpython(&source, "managed_await_footer.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn nested_guards_preserve_erased_iterator_footer() {
    for scope in ["def", "async def"] {
        for prefix in ["", "    if flag:\n        assert False\n"] {
            for condition in ["other", "other is True", "other == 'value'"] {
                for iterable in ["items", "(1, 2, 3)"] {
                    for depth in [1, 2] {
                        let nested = "    ".repeat(depth);
                        let mut source =
                            format!("{scope} sample(flag, other, items):\n{prefix}    if flag:\n");
                        for level in 0..depth {
                            writeln!(source, "{}if {condition}:", "    ".repeat(level + 2))
                                .unwrap();
                        }
                        write!(source, "{nested}        for item in {iterable}:\n{nested}            assert False\n").unwrap();
                        assert_matches_cpython(&source, "nested_iterator_footer.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn erased_branches_preserve_manager_cleanup_predecessors() {
    for (scope, manager) in [
        ("async def", "async with manager"),
        ("def", "with manager"),
        ("async def", "with manager"),
    ] {
        for prefix in ["", "        if flag:\n            assert False\n"] {
            for nested in [false, true] {
                for suffix in ["", "        operation()\n"] {
                    let mut source = format!(
                        "{scope} sample(flag, other, manager, operation):\n    {manager}:\n{prefix}"
                    );
                    let indent = if nested {
                        source.push_str("        if flag:\n            operation()\n        elif other:\n            assert False\n        else:\n");
                        "            "
                    } else {
                        "        "
                    };
                    for condition in ["other", "other == 'value'"] {
                        let source = format!(
                            "{source}{indent}if flag:\n{indent}    assert False\n{indent}if {condition}:\n{indent}    assert False\n{indent}else:\n{indent}    assert False\n{suffix}"
                        );
                        assert_matches_cpython(&source, "erased_manager_predecessors.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn erased_try_guards_preserve_finalizer_manager_cleanup() {
    for scope in ["def", "async def"] {
        for managers in ["manager", "manager, other, third"] {
            for condition in ["flag", "condition()", "flag == 'value'"] {
                for cleanup in ["cleanup()", "cleanup.close()"] {
                    let source = format!(
                        "{scope} sample(manager, other, third, flag, condition, cleanup):\n    with {managers}:\n        try:\n            if {condition}:\n                assert False\n        finally:\n            {cleanup}\n"
                    );
                    assert_matches_cpython(&source, "erased_finalizer_manager.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn erased_try_body_retains_normal_finally_none_owner() {
    for scope in ["def", "async def"] {
        for prefix in [
            "",
            "        if flag:\n            consume(value)\n        else:\n            consume(other)\n",
        ] {
            for assertions in [
                "            assert flag\n",
                "            assert flag\n            assert other\n",
            ] {
                let source = format!(
                    "{scope} f(value, flag, other):\n    with manager():\n{prefix}        try:\n{assertions}        finally:\n            if value is not None:\n                value.close()\n"
                );
                assert_matches_cpython(&source, "erased_try_none_owner.py", &LEVELS);
            }
        }
    }
}

#[test]
fn terminal_raises_preserve_original_conditional_footer() {
    for scope in ["def", "async def"] {
        for prefix in [
            "",
            "    if other:\n        assert other\n",
            "    if other:\n        consume(other)\n",
            "    with manager:\n        consume(other)\n    if other:\n        assert other\n",
        ] {
            for leaf in [
                "raise ValueError(other)",
                "raise error from other",
                "raise error",
                "raise",
            ] {
                for condition in ["not flag", "flag and other"] {
                    let source = format!(
                        "{scope} f(flag, other, error, manager):\n{prefix}    if {condition}:\n        {leaf}\n"
                    );
                    assert_matches_cpython(&source, "terminal_raise_footer.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn terminal_none_returns_preserve_original_conditional_footer() {
    for scope in ["def", "async def"] {
        for prefix in [
            "",
            "    if other:\n        assert other\n",
            "    if other:\n        consume(other)\n",
        ] {
            for leaf in ["return", "return None"] {
                for assertion in ["", "        assert flag\n"] {
                    for condition in ["flag", "not flag", "flag is None", "consume(flag)"] {
                        for suffix in ["", "    assert other\n", "    pass\n"] {
                            let source = format!(
                                "{scope} f(flag, other, consume):\n{prefix}    if {condition}:\n{assertion}        {leaf}\n{suffix}"
                            );
                            assert_matches_cpython(
                                &source,
                                "terminal_none_return_footer.py",
                                &LEVELS,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn named_container_iterators_preserve_conditional_footer() {
    for scope in ["def", "async def"] {
        for prefix in ["", "    if other:\n        assert other\n"] {
            for iterable in [
                "(first, second, 'item')",
                "[first, second, 'item']",
                "(first,)",
                "[first]",
                "(first, (1, 2))",
                "[first, (1, 2)]",
            ] {
                let source = format!(
                    "{scope} f(flag, other, first, second):\n{prefix}    if flag != 3:\n        for item in {iterable}:\n            assert item\n"
                );
                assert_matches_cpython(&source, "named_container_iterator_footer.py", &LEVELS);
            }
        }
    }
}

#[test]
fn literal_iterators_keep_original_conditional_footer() {
    for scope in ["def", "async def"] {
        for prefix in [
            "",
            "    if other:\n        assert other\n",
            "    if other:\n        consume(other)\n",
            "    with manager:\n        consume(other)\n    if other:\n        assert other\n",
        ] {
            for iterable in ["(1, 2)", "()", "[1, 2]", "[]"] {
                for leaf in ["assert value", "consume(value)"] {
                    let source = format!(
                        "{scope} f(flag, other, manager):\n{prefix}    if flag:\n        for value in {iterable}:\n            {leaf}\n"
                    );
                    assert_matches_cpython(&source, "literal_iterator_footer.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn erased_else_guards_preserve_original_paired_call_footer() {
    for scope in ["def", "async def"] {
        for prefix in [
            "",
            "    if other:\n        assert other\n",
            "    if other:\n        consume(other)\n",
            "    with manager:\n        consume(other)\n    if other:\n        assert other\n",
        ] {
            for (left, right) in [
                ("consume(flag)\n", "consume(other)\n"),
                ("value = flag\n", "value = other\n"),
                (
                    "obj.consume(flag)\n",
                    "obj.consume(other)\nconsume(third)\n",
                ),
                (
                    "value = flag\nconsume(value)\n",
                    "value = other\nconsume(value)\n",
                ),
            ] {
                for else_prefix in ["", "if third:\n    assert third\n"] {
                    let mut source = format!(
                        "{scope} f(flag, other, third, obj, manager):\n{prefix}    if flag:\n"
                    );
                    for line in left.lines() {
                        writeln!(source, "        {line}").unwrap();
                    }
                    source.push_str("    else:\n");
                    for line in format!("{else_prefix}{right}").lines() {
                        writeln!(source, "        {line}").unwrap();
                    }
                    assert_matches_cpython(&source, "paired_call_footer.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn single_loop_origin_preserves_protected_finalizer_join() {
    let indent = |source: &str| {
        let mut result = String::new();
        for line in source.lines() {
            writeln!(result, "    {line}").unwrap();
        }
        result
    };
    for prefix in ["", "async "] {
        for header in ["for item in items:", "while ready:"] {
            for manager in ["none", "outer", "inner"] {
                if !prefix.is_empty() && manager != "none" {
                    continue;
                }
                for test in ["flag", "flag and value"] {
                    let mut body = format!(
                        "try:\n    if {test}:\n        assert value\n    else:\n        assert not value\nfinally:\n    callback()\n"
                    );
                    if manager == "inner" {
                        body = format!("with manager:\n{}", indent(&body));
                    }
                    body = format!("{header}\n{}", indent(&body));
                    if manager == "outer" {
                        body = format!("with manager:\n{}", indent(&body));
                    }
                    let source = format!(
                        "{prefix}def sample(flag, value, ready, items, manager, callback):\n{}",
                        indent(&body)
                    );
                    assert_matches_cpython(&source, "single_loop_finalizer_join.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn pass_match_success_preserves_normal_try_fallthrough() {
    for scope in ["def", "async def"] {
        for prefix in ["", "with manager:\n    value = produce()\n"] {
            for cases in [
                "case 1:\n    consume(value)\ncase _:\n    consume(value)\n",
                "case 1:\n    pass\ncase _:\n    consume(value)\n",
                "case A():\n    consume(value)\ncase _:\n    consume(value)\n",
                "case A():\n    pass\ncase _:\n    consume(value)\n",
                "case A():\n    pass\ncase B() | C():\n    consume(value)\ncase _:\n    consume(value)\n",
                "case A():\n    consume(value)\ncase B() | C():\n    consume(value)\ncase _:\n    consume(value)\n",
                "case A():\n    pass\ncase B() | C():\n    consume(value)\ncase _:\n    obj.consume(value)\n",
                "case A():\n    pass\ncase B() | C():\n    consume(value)\ncase _:\n    consume(value, mode=\"value\")\n",
            ] {
                for handler in ["consume(value)\n", "consume(value)\nraise\n"] {
                    let mut source = format!("{scope} f(value, obj, manager):\n    try:\n");
                    for line in prefix.lines() {
                        writeln!(source, "        {line}").unwrap();
                    }
                    source.push_str("        match value:\n");
                    for line in cases.lines() {
                        writeln!(source, "            {line}").unwrap();
                    }
                    source.push_str("    except Exception:\n");
                    for line in handler.lines() {
                        writeln!(source, "        {line}").unwrap();
                    }
                    source.push_str("    consume(value)\n");
                    assert_matches_cpython(&source, "pass_match_try.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn direct_manager_entry_preserves_elided_protected_join() {
    for (prefix, manager_keyword) in [("", "with"), ("async ", "with"), ("async ", "async with")] {
        for manager in ["manager", "open_manager(value)"] {
            for test in ["flag", "flag and other"] {
                for body in ["pass", "consume(value)"] {
                    for guarded_cleanup in [false, true] {
                        let cleanup = if prefix.is_empty() {
                            "cleanup(value)"
                        } else {
                            "await cleanup(value)"
                        };
                        let cleanup = if guarded_cleanup {
                            format!("        if other:\n            {cleanup}\n")
                        } else {
                            format!("        {cleanup}\n")
                        };
                        let source = format!(
                            "{prefix}def sample(flag, value, other, manager):\n    try:\n        if {test}:\n            assert value\n        else:\n            assert not value\n        {manager_keyword} {manager}:\n            {body}\n    finally:\n{cleanup}"
                        );
                        assert_matches_cpython(&source, "direct_manager_entry_join.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn manager_assignment_target_retains_its_protected_region() {
    for (prefix, keyword) in [("", "with"), ("async ", "with"), ("async ", "async with")] {
        for target in ["holder.value", "(holder if flag else alternate).value"] {
            for finalizer in [false, true] {
                let mut source = format!("{prefix}def sample(manager, holder, alternate, flag):\n");
                let indent = if finalizer {
                    source.push_str("    try:\n");
                    "        "
                } else {
                    "    "
                };
                writeln!(source, "{indent}{keyword} manager as {target}:").unwrap();
                writeln!(source, "{indent}    pass").unwrap();
                if finalizer {
                    source.push_str("    finally:\n        cleanup()\n");
                }
                assert_matches_cpython(&source, "manager_assignment_target.py", &LEVELS);
            }
        }
    }
}

#[test]
fn unmanaged_enclosing_finalizer_preserves_live_loop_join() {
    for scope in ["def", "async def"] {
        for header in ["for item in items:", "while ready:"] {
            for condition in ["flag", "flag and value"] {
                for successor in ["consume(value)", "value = other"] {
                    let source = format!(
                        "{scope} sample(flag, value, other, ready, items):\n    try:\n        {header}\n            if {condition}:\n                assert value\n            else:\n                assert not value\n            {successor}\n    finally:\n        cleanup(value)\n"
                    );
                    assert_matches_cpython(&source, "enclosing_finalizer_loop_join.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn terminal_async_break_preserves_normal_finally_borrow_boundary() {
    let indent = |source: &str| {
        let mut result = String::new();
        for line in source.lines() {
            writeln!(result, "    {line}").unwrap();
        }
        result
    };
    for (prefix_index, prefix) in [
        "",
        "break\n",
        "return\n",
        "if value:\n    continue\n",
        "try:\n    await consume(item)\nexcept Other:\n    pass\n",
        "try:\n    consume(item)\nexcept Other:\n    await recover(value)\n",
        "if value:\n    return\n",
        "continue\n",
    ]
    .into_iter()
    .enumerate()
    {
        for (handler_index, handler) in [
            "pass\n",
            "await recover(value)\n",
            "assert flag == value\nvalue = flag\n",
        ]
        .into_iter()
        .enumerate()
        {
            for (cleanup_index, cleanup) in [
                "cleanup(value)\n",
                "await cleanup(value)\n",
                "cleanup(await resolve(value))\n",
                "await cleanup(await resolve(value))\n",
            ]
            .into_iter()
            .enumerate()
            {
                let protected = format!(
                    "async for item in source:\n{}",
                    indent(&format!("{prefix}if flag:\n    break\n"))
                );
                let suite = format!(
                    "try:\n{}except Exception:\n{}finally:\n{}finish(value)\n",
                    indent(&protected),
                    indent(handler),
                    indent(cleanup)
                );
                assert_matches_cpython(
                    &format!("async def f(value, source, flag):\n{}", indent(&suite)),
                    &format!(
                        "async_break_finally_{prefix_index}_{handler_index}_{cleanup_index}.py"
                    ),
                    &LEVELS,
                );
            }
        }
    }
}

#[test]
fn erased_assertions_preserve_normal_finally_conditional_exits() {
    for (scope_index, (scope, prefix)) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield 0\n"),
        ("async def", "    yield 0\n"),
    ]
    .into_iter()
    .enumerate()
    {
        for (shape_index, finalbody) in [
            "if flag:\n    consume()\nassert third\n",
            "if flag:\n    if other:\n        consume()\n    assert third\n",
            "if flag:\n    value = other\n    consume(value)\n    if third:\n        consume()\n    assert other\n",
            "value = other\nif flag:\n    value = third\n    consume(value)\n    if value is not None:\n        if flag and other:\n            consume()\n        assert third\n",
        ]
        .into_iter()
        .enumerate()
        {
            let mut source = format!(
                "{scope} f(flag, other, third):\n{prefix}    try:\n        consume()\n    finally:\n"
            );
            for line in finalbody.lines() {
                writeln!(source, "        {line}").unwrap();
            }
            assert_matches_cpython(
                &source,
                &format!("normal_finally_conditional_{scope_index}_{shape_index}.py"),
                &LEVELS,
            );
        }
    }
}

#[test]
fn erased_else_preserves_original_linear_footer() {
    for scope in ["def", "async def"] {
        for prefix in [
            "",
            "    if other:\n        assert other\n",
            "    if other:\n        consume(other)\n",
            "    with manager:\n        consume(other)\n    if other:\n        assert other\n",
        ] {
            for leaf in [
                "value = other",
                "consume(other)",
                "value = consume(other)",
                "obj.value = other",
            ] {
                for suffix in ["", "    assert third\n"] {
                    let source = format!(
                        "{scope} f(flag, other, third, obj, manager):\n{prefix}    if flag:\n        {leaf}\n        assert third\n    else:\n        assert other\n{suffix}"
                    );
                    assert_matches_cpython(&source, "erased_else_linear_footer.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn erased_protected_arms_keep_finalizer_entry_before_nonterminal_control_flow() {
    let indent = |source: &str| {
        let mut result = String::new();
        for line in source.lines() {
            writeln!(result, "    {line}").unwrap();
        }
        result
    };
    for prefix in ["", "async "] {
        for first in [
            "consume()",
            "callback()",
            "manager.release()",
            "(lambda: None)()",
            "await callback()",
            "await value",
        ] {
            if prefix.is_empty() && first.starts_with("await ") {
                continue;
            }
            for tail in [
                "if value:\n    consume()\n",
                "for item in items:\n    consume(item)\n",
                "while value:\n    consume()\n    break\n",
                "with manager:\n    consume()\n",
                "try:\n    consume()\nexcept Exception:\n    consume()\n",
            ] {
                for test in ["flag", "flag and value"] {
                    let finalizer = format!("{first}\n{tail}");
                    let body = format!(
                        "try:\n    if {test}:\n        assert value\n    else:\n        assert not value\nfinally:\n{}",
                        indent(&finalizer)
                    );
                    let source = format!(
                        "{prefix}def sample(flag, value, manager, items, callback):\n{}",
                        indent(&body)
                    );
                    assert_matches_cpython(&source, "erased_finalizer_entry.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn ordinary_awaits_keep_original_scope_footer_after_erased_assertions() {
    for prefix in [
        "",
        "    if other:\n        assert other\n",
        "    if other:\n        consume(other)\n",
        "    with manager:\n        consume(other)\n    if other:\n        assert other\n",
    ] {
        for leaf in [
            "        await consume(flag)\n",
            "        value = flag\n        await consume(value)\n",
            "        await consume(other)\n        await consume(flag)\n",
            "        consume(other)\n        await consume(flag)\n",
        ] {
            for (inner, outer) in [
                ("", ""),
                ("        assert third\n", ""),
                ("", "    assert third\n"),
                ("        assert third\n", "    assert third\n"),
            ] {
                let source = format!(
                    "async def f(flag, other, third, manager):\n{prefix}    if flag:\n{leaf}{inner}{outer}"
                );
                assert_matches_cpython(&source, "ordinary_await_scope_footer.py", &LEVELS);
            }
        }
    }
}

#[test]
fn conjunction_assertions_preserve_original_call_footer() {
    for scope in ["def", "async def"] {
        for prefix in [
            "",
            "    if other:\n        assert other\n",
            "    if other:\n        consume(other)\n",
            "    with manager:\n        consume(other)\n    if other:\n        assert other\n",
        ] {
            for test in [
                "flag and other",
                "flag and other and third",
                "flag is None and other",
                "flag and other is None",
            ] {
                for leaf in [
                    "        consume(flag)\n",
                    "        value = flag\n        consume(value)\n",
                ] {
                    let source = format!(
                        "{scope} f(flag, other, third, manager):\n{prefix}    if {test}:\n{leaf}        assert third\n    assert third\n"
                    );
                    assert_matches_cpython(&source, "conjunction_call_footer.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn erased_assertions_preserve_original_call_footer() {
    for scope in ["def", "async def"] {
        for prefix in [
            "",
            "    if other:\n        assert other\n",
            "    if other:\n        consume(other)\n",
            "    with manager:\n        consume(other)\n    if other:\n        assert other\n",
        ] {
            for test in ["flag", "flag is None"] {
                for call in ["consume()", "obj.consume()"] {
                    for suffix in ["    assert flag\n", "    return None\n"] {
                        let source = format!(
                            "{scope} f(flag, other, obj, manager):\n{prefix}    if {test}:\n        {call}\n{suffix}"
                        );
                        assert_matches_cpython(&source, "erased_call_footer.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn erased_protected_arms_keep_resumable_join() {
    let indent = |source: &str| {
        let mut result = String::new();
        for line in source.lines() {
            writeln!(result, "    {line}").unwrap();
        }
        result
    };
    for (prefix, suspension) in [
        ("", ""),
        ("async ", ""),
        ("", "yield 1\n"),
        ("async ", "yield 1\n"),
    ] {
        for manager in ["none", "outer", "inner"] {
            for test in ["flag", "flag and value"] {
                let mut body = format!(
                    "if {test}:\n    assert value\nelse:\n    assert not value\nconsume()\n"
                );
                if manager == "inner" {
                    body = format!("with manager:\n{}", indent(&body));
                }
                body = format!("try:\n{}except Exception:\n    consume()\n", indent(&body));
                if manager == "outer" {
                    body = format!("with manager:\n{}", indent(&body));
                }
                let source = format!(
                    "{prefix}def sample(flag, value, manager):\n{}{}",
                    indent(suspension),
                    indent(&body)
                );
                assert_matches_cpython(&source, "erased_protected_resumable_join.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_manager_arms_keep_protected_finalizer_join() {
    let indent = |source: &str| {
        let mut result = String::new();
        for line in source.lines() {
            writeln!(result, "    {line}").unwrap();
        }
        result
    };
    for prefix in ["", "async "] {
        for managers in ["manager", "manager, manager"] {
            for test in ["flag", "flag and value"] {
                for suffix in ["", "consume()\n"] {
                    let body = format!(
                        "if {test}:\n    assert value\nelse:\n    assert not value\n{suffix}"
                    );
                    let protected = format!("with {managers}:\n{}", indent(&body));
                    let body = format!("try:\n{}finally:\n    consume()\n", indent(&protected));
                    let source = format!(
                        "{prefix}def sample(flag, value, manager):\n{}",
                        indent(&body)
                    );
                    assert_matches_cpython(&source, "erased_manager_finalizer_join.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn coroutine_terminal_loops_keep_original_scope_footer() {
    for (prefix_index, prefix) in [
        "",
        "    if flag:\n        assert flag\n",
        "    if flag:\n        consume(flag)\n",
        "    await consume()\n    if flag:\n        assert flag\n",
    ]
    .into_iter()
    .enumerate()
    {
        for (iterator_index, iterator) in ["values", "obj.values", "iter(values)"]
            .into_iter()
            .enumerate()
        {
            for (body_index, body) in [
                "            assert item\n",
                "            consume(item)\n",
                "            if flag:\n                continue\n            consume(item)\n",
            ]
            .into_iter()
            .enumerate()
            {
                let source = format!(
                    "async def f(flag, values, obj):\n{prefix}    if flag:\n        for item in {iterator}:\n{body}"
                );
                assert_matches_cpython(
                    &source,
                    &format!(
                        "coroutine_terminal_loop_{prefix_index}_{iterator_index}_{body_index}.py"
                    ),
                    &LEVELS,
                );
            }
        }
    }
}

#[test]
fn erased_loop_assignment_suffix_keeps_original_scope_footer() {
    for scope in ["def", "async def"] {
        for prefix in [
            "",
            "    if flag:\n        assert flag\n",
            "    if flag:\n        consume(flag)\n",
        ] {
            for iterator in ["values", "obj.values"] {
                for value in ["item", "item.value", "item[0]"] {
                    for suffix in [
                        "            assert result\n",
                        "            assert result\n            assert flag\n",
                    ] {
                        let source = format!(
                            "{scope} f(flag, values, obj, consume):\n{prefix}    if flag:\n        for item in {iterator}:\n            result = {value}\n{suffix}"
                        );
                        assert_matches_cpython(&source, "loop_assignment_footer.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn erased_boolean_arms_preserve_resumable_scope_footer() {
    for (scope, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield 0\n"),
        ("async def", "    yield 0\n"),
    ] {
        for test in [
            "flag or other",
            "flag or other or third",
            "flag is None or other or third",
            "flag and other or third",
        ] {
            for prefix in [
                "",
                "    consume()\n",
                "    if third:\n        assert third\n",
                "    with manager:\n        consume()\n",
            ] {
                let source = format!(
                    "{scope} f(flag, other, third, manager):\n{suspension}{prefix}    if {test}:\n        assert third\n"
                );
                assert_matches_cpython(&source, "erased_boolean_footer.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_paired_branch_suffix_keeps_each_return_donor() {
    for declaration in ["def", "async def"] {
        for (left, right) in [
            ("consume(flag)", "consume(other)"),
            ("value = flag", "value = other"),
            ("obj.consume(flag)", "obj.consume(other)"),
        ] {
            let branch = format!("if other:\n    {left}\nelse:\n    {right}\nassert third\n");
            let nested = branch.replace('\n', "\n    ");
            for body in [
                branch.clone(),
                format!("if flag:\n    {nested}"),
                format!("if flag:\n    consume(third)\nelse:\n    {nested}"),
            ] {
                let source = format!(
                    "{declaration} f(flag, other, third, obj):\n    {}",
                    body.replace('\n', "\n    ")
                );
                assert_matches_cpython_versions(
                    &source,
                    "erased_paired_branch_suffix.py",
                    &LEVELS,
                    &[
                        PythonVersion::Py313,
                        PythonVersion::Py314,
                        PythonVersion::Py315,
                    ],
                );
            }
        }
    }
}

#[test]
fn erased_coroutine_paired_branches_preserve_copied_footer_donors() {
    let source = include_str!("fixtures/coroutine_paired_branch_footers.py");
    for source in [
        source.to_owned(),
        source.replace("assert flag", "action(flag)"),
        source.replace("        assert flag\n", ""),
        source.replace("await ", ""),
        source.replace("async def", "def").replace("await ", ""),
    ] {
        assert_matches_cpython_versions(
            &source,
            "coroutine_paired_branch_footers.py",
            &LEVELS,
            &[
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ],
        );
    }
    assert_matches_cpython_versions(
        &source.replace("assert flag", "pass"),
        "coroutine_paired_branch_footers.py",
        &LEVELS,
        &[PythonVersion::Py313],
    );
}

#[test]
fn completed_coroutine_prefix_preserves_comprehension_footer_donors() {
    let source = include_str!("fixtures/coroutine_completed_comprehension_footers.py");
    assert_matches_cpython_versions(
        source,
        "coroutine_completed_comprehension_footers.py",
        &LEVELS,
        &[PythonVersion::Py314, PythonVersion::Py315],
    );
}

#[test]
fn erased_protected_arms_keep_synchronous_for_finalizer_entry() {
    let indent = |source: &str| {
        let mut result = String::new();
        for line in source.lines() {
            writeln!(result, "    {line}").unwrap();
        }
        result
    };
    for prefix in ["", "async "] {
        for iterable in ["items", "callback()", "manager.items()", "(1, 2)"] {
            for loop_body in ["consume(item)", "pass", "break", "continue"] {
                for test in ["flag", "flag and value"] {
                    let finalizer = format!("for item in {iterable}:\n    {loop_body}\n");
                    let body = format!(
                        "try:\n    if {test}:\n        assert value\n    else:\n        assert not value\nfinally:\n{}",
                        indent(&finalizer)
                    );
                    let source = format!(
                        "{prefix}def sample(flag, value, manager, items, callback):\n{}",
                        indent(&body)
                    );
                    assert_matches_cpython(&source, "erased_for_finalizer_entry.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn async_for_setups_preserve_repeated_scope_footer() {
    let source = include_str!("fixtures/async_for_original_scope_footer.py");
    for (index, source) in [
        source.to_owned(),
        source.replace("async for", "for"),
        source.replace("break", "continue"),
        source.replace("break", "pass"),
        source.replace("        assert second", "        consume(second)"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("async_for_original_scope_footer_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn async_for_recovery_preserves_legacy_scope_footer() {
    assert_matches_cpython_versions(
        include_str!("fixtures/async_for_recovered_scope_footer.py"),
        "async_for_recovered_scope_footer.py",
        &LEVELS,
        &[PythonVersion::Py312, PythonVersion::Py313],
    );
}

#[test]
fn coroutine_terminal_loops_keep_suspension_and_cleanup_boundaries() {
    for (index, body) in [
        "    if flag:\n        async for item in values:\n            consume(item)\n",
        "    if flag:\n        for item in values:\n            await consume(item)\n",
        "    if flag:\n        for item in await consume(values):\n            consume(item)\n",
        "    if flag:\n        for item in values:\n            consume(item)\n        else:\n            consume(flag)\n",
        "    if flag:\n        for item in values:\n            break\n",
        "    if flag:\n        for item in values:\n            return item\n",
        "    if flag:\n        for item in values:\n            yield item\n",
        "    if flag:\n        for item in (values if flag else obj.values):\n            consume(item)\n",
        "    if flag:\n        for obj.value in values:\n            consume(obj.value)\n",
        "    if flag:\n        for (item, (left, right)) in values:\n            consume(item)\n",
    ]
    .into_iter()
    .enumerate()
    {
        for (variant, (prefix, suffix)) in [
            ("", ""),
            ("    if flag:\n        assert flag\n", "    return\n"),
        ]
        .into_iter()
        .enumerate()
        {
            let source = format!("async def f(flag, values, obj):\n{prefix}{body}{suffix}");
            assert_matches_cpython(
                &source,
                &format!("coroutine_terminal_loop_boundary_{index}_{variant}.py"),
                &LEVELS,
            );
        }
    }
    for (index, body) in [
        "    with manager():\n        if flag:\n            for item in values:\n                consume(item)\n",
        "    try:\n        if flag:\n            for item in values:\n                consume(item)\n    finally:\n        consume(flag)\n",
        "    try:\n        consume(flag)\n    except ValueError:\n        if flag:\n            for item in values:\n                consume(item)\n",
        "    for value in values:\n        if flag:\n            for item in values:\n                consume(item)\n",
        "    if await consume(flag):\n        for item in values:\n            consume(item)\n",
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!(
            "async def f(flag, values, obj):\n    if flag:\n        assert flag\n{body}"
        );
        assert_matches_cpython(
            &source,
            &format!("coroutine_terminal_loop_protected_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn manager_entry_conditionals_keep_original_array_owners() {
    assert_matches_cpython(
        include_str!("fixtures/manager_entry_conditional_owners.py"),
        "manager_entry_conditional_owners.py",
        &LEVELS,
    );
}

#[test]
fn runtime_list_conditionals_keep_original_array_owners() {
    assert_matches_cpython(
        include_str!("fixtures/runtime_list_conditional_owners.py"),
        "runtime_list_conditional_owners.py",
        &LEVELS,
    );
}

#[test]
fn retained_loop_constant_returns_keep_original_finally_owners() {
    assert_matches_cpython(
        include_str!("fixtures/retained_loop_constant_finally_owners.py"),
        "retained_loop_constant_finally_owners.py",
        &LEVELS,
    );
}

#[test]
fn erased_nested_arms_keep_the_enclosing_iterator_exit() {
    let source = r"def process(items, flags, kinds, width):
    with manager() as owner, manager(), manager(), manager(), manager(), manager():
        for entry in items:
            if flags & kinds.first:
                assert entry.left is not None
                assert entry.width == width
                if len(entry.left.shape) > 3:
                    assert entry.left.shape[1] == width
                    assert entry.right.shape[1] == width
            else:
                assert entry.left is None
            assert entry.valid
            if flags & kinds.second:
                assert entry.left is not None
                assert entry.width == width
                if len(entry.right.shape) > 3:
                    assert entry.left.shape[1] == width
                    assert entry.right.shape[1] == width
            else:
                assert entry.right is None
        result = transform(items)
";
    for source in [
        source.to_owned(),
        source.replace(
            "with manager() as owner, manager(), manager(), manager(), manager(), manager():",
            "with manager():",
        ),
    ] {
        assert_matches_cpython(&source, "erased_nested_iterator_arm.py", &LEVELS);
    }
}

#[test]
fn nested_erased_joins_inherit_conditional_successors() {
    let source = r"def process(items, count, flags, kinds, enabled, limit):
    with manager() as owner, manager():
        for index, entry in enumerate(items):
            if count > 0 and flags & kinds.first and enabled:
                if index == limit:
                    assert entry.left is not None
                    assert entry.width == count
                else:
                    assert entry.left is None
            assert entry.valid
            if count > 0 and flags & kinds.second:
                assert entry.right is not None
            else:
                assert entry.right is None
";
    for source in [
        source.to_owned(),
        source.replace("with manager() as owner, manager():", "with manager():"),
        source.replace(
            "for index, entry in enumerate(items):",
            "while ready(items):\n            index, entry = next(items)",
        ),
    ] {
        assert_matches_cpython(&source, "nested_conditional_successor.py", &LEVELS);
    }
}

#[test]
fn nested_erased_loop_joins_keep_the_manager_owner() {
    let source = r"def process(items, count, flags, kinds, enabled):
    with manager() as first, manager() as second, manager():
        for entry in items:
            if count > 0 and flags & kinds.first:
                assert entry.left is not None
                if enabled:
                    assert entry.scale == 128 / count
                else:
                    assert entry.scale == 1
            else:
                assert entry.left is None
            assert entry.valid
            if count > 0 and flags & kinds.second:
                assert entry.right is not None
            else:
                assert entry.right is None
";
    for source in [
        source.to_owned(),
        source.replace(
            "with manager() as first, manager() as second, manager():",
            "with manager():",
        ),
        source.replace(
            "for entry in items:",
            "while ready(items):\n            entry = next(items)",
        ),
    ] {
        assert_matches_cpython(&source, "nested_erased_loop_join.py", &LEVELS);
    }
}

#[test]
fn async_manager_preserves_conditional_join_before_outer_finalizer() {
    let source = r"async def f(manager, stream, enabled, expected):
    with manager:
        try:
            async with stream:
                result = await receive()
                if enabled and transform(result).value == expected:
                    assert result.valid
                else:
                    assert not result.valid
                data = (await receive()).read()
                consume(data)
                for item in data:
                    consume(item)
        finally:
            try:
                if stream is not None:
                    await cleanup(stream)
            finally:
                await finish()
";
    for source in [
        source.to_owned(),
        source.replace("    with manager:", "    async with manager:"),
        source.replace("            async with stream:", "            async with stream, other:"),
        source.replace(
            "        finally:\n            try:\n                if stream is not None:\n                    await cleanup(stream)\n            finally:\n                await finish()",
            "        finally:\n            await finish()",
        ),
    ] {
        assert_matches_cpython(&source, "async_manager_finally_join.py", &LEVELS);
    }
}

#[test]
fn populated_elif_chains_preserve_conditional_loop_join() {
    let source = r#"def collect(groups, first, second):
    for group in groups:
        for batch in group:
            for item in batch:
                if item.kind == 1:
                    if first:
                        value = f"first_{item.part}"
                    else:
                        value = "first"
                elif item.kind == 2:
                    assert first
                    if second:
                        value = f"second_{item.part}"
                    elif first:
                        value = f"second_{item.other}"
                    else:
                        value = "second"
                else:
                    assert False
                if item.ready:
                    emit(value)
"#;
    for source in [
        source.to_owned(),
        source.replace("                    assert first\n", ""),
        source.replace(
            "                if item.ready:\n                    emit(value)",
            "                emit(value)",
        ),
        source.replace(
            "                    elif first:\n                        value = f\"second_{item.other}\"\n",
            "",
        ),
        source.replace(
            "                    else:\n                        value = \"second\"",
            "                    elif item.valid:\n                        value = \"third\"\n                    else:\n                        value = \"second\"",
        ),
        source.replace(
            "                        value = f\"second_{item.other}\"",
            "                        assert first",
        ),
        source.replace(
            "                        value = f\"second_{item.other}\"",
            "                        value = [part for part in item.parts]",
        ),
    ] {
        assert_matches_cpython(&source, "populated_chain_loop_join.py", &LEVELS);
    }
}

#[test]
fn protected_nested_joins_keep_their_finally_owner() {
    let source = r"async def collect(items, enabled):
    try:
        if enabled:
            if ready():
                values = {entry.key: entry for entry in items}
            else:
                report('missing')
        else:
            assert valid()
        with manager() as stream:
            consume(stream)
        yield values
    finally:
        await cleanup()
";
    for source in [
        source.to_owned(),
        source.replace("yield values", "await process(values)"),
        source
            .replace("async def", "def")
            .replace("await cleanup()", "cleanup()"),
        source
            .replace("async def", "def")
            .replace("yield values", "consume(values)")
            .replace("await cleanup()", "cleanup()"),
        source.replace(
            "        with manager() as stream:\n            consume(stream)",
            "        consume(values)",
        ),
    ] {
        assert_matches_cpython(&source, "protected_nested_join.py", &LEVELS);
    }
}

#[test]
fn erased_elif_chains_preserve_live_loop_join() {
    let source = r"def collect(items, kinds, enabled):
    for value in items:
        if enabled:
            if value.kind == kinds.first:
                assert value.valid
            elif value.kind == kinds.second:
                assert value.valid
            elif value.kind == kinds.third:
                assert value.valid
            else:
                assert False
        else:
            assert value.kind in kinds.allowed
        consume(value)
    finish()
";
    for source in [
        source.to_owned(),
        source.replace("for value in items:", "while ready(items):"),
        source.replace(
            "    finish()",
            "    else:\n        completed()\n    finish()",
        ),
        source.replace("        consume(value)", "        output = value"),
        source.replace("        consume(value)", "        assert value.valid"),
    ] {
        assert_matches_cpython(&source, "erased_chain_loop_join.py", &LEVELS);
    }
}

#[test]
fn enclosing_finalizer_preserves_live_loop_join() {
    let source = r"def process(items, owner):
    try:
        context = owner.context()
        with context:
            for item in items:
                prepare(item)
                if owner.enabled():
                    if ready():
                        assert context.valid
                    else:
                        assert context.other
                    context.reset()
        finish()
    finally:
        heartbeat()
        with timeout():
            owner.close()
    finish_all()
";
    for source in [
        source.to_owned(),
        source.replace(
            "        heartbeat()\n        with timeout():\n            owner.close()\n",
            "        cleanup()\n",
        ),
        source.replace("for item in items:", "while owner.more():"),
        source.replace("if ready():", "if ready() or other():"),
        source.replace(
            "                    context.reset()",
            "                    assert context.valid",
        ),
    ] {
        assert_matches_cpython(&source, "finally_loop_join.py", &LEVELS);
    }
}

#[test]
fn nested_erased_condition_keeps_live_loop_successor() {
    for successor in ["emit(item)", "result = transform(item)"] {
        let source = format!(
            "def f(items, flag):\n    with manager():\n        for item in items:\n            if flag:\n                if item.ready:\n                    assert item\n                else:\n                    assert flag\n            {successor}\n"
        );
        assert_matches_cpython(&source, "live_loop_successor.py", &LEVELS);
    }
}

#[test]
fn nested_erased_conditions_preserve_manager_join() {
    for managers in ["first(), second()", "first(), second(), third()"] {
        let source = format!(
            "async def f(flag, other):\n    with {managers}:\n        result = await work()\n        if flag:\n            assert result\n            if other:\n                assert result\n            else:\n                assert other\n        else:\n            assert flag\n"
        );
        assert_matches_cpython(&source, "manager_join.py", &LEVELS);
    }
}

#[test]
fn erased_iterator_else_preserves_optional_inner_branch() {
    for definition in ["def", "async def"] {
        for body in [
            "                if magnitude(item) / scale(item) > limit:\n                    output.emit(item)\n",
            "                if item:\n                    output.emit(item)\n",
            "                if item:\n                    output.emit(item)\n                assert item\n",
            "                if item:\n                    output.emit(item)\n                else:\n                    consume(item)\n",
        ] {
            let source = format!(
                "{definition} f(manager, items, output, limit):\n    with manager:\n        for item in items:\n            if first(item):\n                consume(item)\n            elif accepts(item):\n                assert item\n{body}            else:\n                assert False\n"
            );
            assert_matches_cpython(&source, "optional_inner_branch.py", &LEVELS);
        }
    }
}

#[test]
fn erased_iterator_else_preserves_conditional_call_argument_join() {
    for definition in ["def", "async def"] {
        for call in [
            "receiver.send(value=item, option=factory() if factory else None)",
            "consume(item, factory() if factory else None)",
            "receiver.send(value=item, option=factory())",
        ] {
            let source = format!(
                "{definition} f(manager, items, receiver, factory):\n    with manager:\n        for item in items:\n            if check(item):\n                consume(item)\n            elif receiver is not None:\n                assert item\n                {call}\n            else:\n                assert factory\n"
            );
            assert_matches_cpython(&source, "conditional_argument_iterator.py", &LEVELS);
        }
    }
}

#[test]
fn erased_inner_iterator_body_preserves_outer_condition_backedge() {
    for definition in ["def", "async def"] {
        for body in ["assert value", "pass", "emit(value)"] {
            let source = format!(
                "{definition} f(manager, groups):\n    with manager:\n        for group in groups:\n            prepare(group)\n            for item in group:\n                if valid(item):\n                    for value in values(item):\n                        {body}\n                else:\n                    assert item\n            finish(group)\n"
            );
            assert_matches_cpython(&source, "erased_inner_iterator.py", &LEVELS);
        }
    }
}

#[test]
fn erased_else_preserves_outer_iterator_after_inner_for() {
    for prefix in ["", "                assert item\n"] {
        let source = format!(
            "def f(items):\n    try:\n        for item in items:\n            if first(item):\n                emit(item)\n            elif second(item) and ready(item):\n{prefix}                for value in values(item):\n                    emit(value)\n            else:\n                assert False\n    except ValueError:\n        raise\n"
        );
        assert_matches_cpython(&source, "outer_iterator.py", &LEVELS);
    }
}

#[test]
fn erased_else_preserves_populated_iterator_elif() {
    for body in [
        "                emit(value)\n",
        "                if flag:\n                    result = left(value)\n                else:\n                    result = right(value)\n                emit(result)\n",
    ] {
        let source = format!(
            "def f(manager, values, flag):\n    with manager:\n        for value in values:\n            if first(value):\n                emit(value)\n                if flag:\n                    emit(value)\n            elif second(value):\n{body}            else:\n                assert False\n"
        );
        assert_matches_cpython(&source, "populated_iterator_elif.py", &LEVELS);
    }
}

#[test]
fn erased_else_preserves_try_while_work() {
    for prefix in ["", "                assert flag\n"] {
        let source = format!(
            "def f(flag):\n    try:\n        while True:\n            if first(flag):\n                emit(flag)\n            elif second(flag):\n{prefix}                result = transform(flag)\n                emit(result)\n            else:\n                assert False\n    except ValueError:\n        pass\n"
        );
        assert_matches_cpython(&source, "try_while_work.py", &LEVELS);
    }
}

#[test]
fn erased_else_preserves_protected_pass_backedge() {
    for definition in ["def", "async def"] {
        for loop_header in ["while True", "for value in flag"] {
            let source = format!(
                "{definition} f(flag):\n    try:\n        {loop_header}:\n            if flag == 1:\n                pass\n            else:\n                assert False\n    finally:\n        print(flag)\n"
            );
            assert_matches_cpython(&source, "protected_pass_backedge.py", &LEVELS);
        }
    }
}

#[test]
fn erased_else_preserves_protected_iterator_continue() {
    for body in [
        "            if value == 1:\n                continue\n            else:\n                assert False\n",
        "            if value == 0:\n                await sink(value)\n            elif value == 1:\n                continue\n            else:\n                assert False\n",
    ] {
        let source = format!(
            "async def f(values):\n    try:\n        async for value in values:\n{body}    finally:\n        print(values)\n",
        );
        assert_matches_cpython(&source, "protected_iterator_continue.py", &LEVELS);
    }
}

#[test]
fn erased_branches_preserve_protected_while_join() {
    for loop_test in ["True", "flag", "flag != stop"] {
        let source = format!(
            "def f(manager, flag, stop):\n    with manager:\n        while {loop_test}:\n            if flag:\n                assert flag\n            else:\n                assert stop\n            if stop:\n                break\n            else:\n                print(flag)\n"
        );
        assert_matches_cpython(&source, "protected_while_join.py", &LEVELS);
    }
}

#[test]
fn annotated_comprehension_preserves_implicit_return_location() {
    let source = "async def collect(items, fields):\n    await prepare()\n    result: list[tuple] = [\n        tuple(item for field in fields)\n        for item in items\n    ]\n    assert result\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("    assert result\n", ""),
        source
            .replace("async def", "def")
            .replace("    await prepare()\n", ""),
        source
            .replace("result: list[tuple] = [", "result: set[tuple] = {")
            .replace("    ]\n", "    }\n"),
        source
            .replace("result: list[tuple] = [", "result: dict[object, tuple] = {")
            .replace(
                "        tuple(item for field in fields)",
                "        item: tuple(item for field in fields)",
            )
            .replace("    ]\n", "    }\n"),
        source.replace("result: list[tuple]", "result"),
        source.replace("    assert result\n", "    consume(result)\n"),
        source.replace("tuple(item for field in fields)", "item"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("annotated_comprehension_return_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_match_default_keeps_manager_exit_outside_exception_regions() {
    let source = "def f(manager, other, value):\n    with manager:\n        match value:\n            case 1:\n                return first()\n            case 2:\n                return second()\n            case _:\n                assert False\n";
    for (case, source) in [
        source.to_string(),
        source.replace("with manager:", "with manager, other:"),
        source.replace("case 1:", "case 1 if value:"),
        source.replace("case _:", "case _ if value:"),
        format!("{source}        finish()\n"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("match_default_manager_{case}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_match_case_suffix_preserves_protected_conditional_join() {
    for managers in ["manager", "manager, other"] {
        for suffix in ["", "        finish()\n"] {
            for body in ["action()", "pass"] {
                let source = format!(
                    "def f(value, ready, manager, other):\n    with {managers}:\n        match value:\n            case 1:\n                if ready:\n                    {body}\n                assert ready\n{suffix}"
                );
                assert_matches_cpython(&source, "protected_match_conditional_join.py", &LEVELS);
            }
        }
    }
}

#[test]
fn compacted_branch_pass_preserves_return_copy_location() {
    for definition in ["def", "async def"] {
        let source = format!(
            "{definition} f(kind, left, right, flag):\n    if kind == left:\n        result = left\n        assert flag\n        pass\n    elif kind == right:\n        result = right\n        if flag:\n            assert kind\n        else:\n            assert kind\n        pass\n    else:\n        raise ValueError(\"kind\")\n    assert result\n"
        );
        assert_matches_cpython(&source, "compacted_branch_pass.py", &LEVELS);
    }
}

#[test]
fn protected_iterator_preserves_completed_conditional_backedges() {
    for definition in ["def", "async def"] {
        for managers in ["manager", "manager, other, third"] {
            let source = format!(
                "{definition} f(manager, other, third, values, active):\n    with {managers}:\n        for value in values:\n            if skip(value):\n                continue\n            item = value.load()\n            if active:\n                assert item\n                if item.kind == 1:\n                    expected = {{1: 2}}\n                else:\n                    expected = {{1: 3}}\n                assert expected\n            else:\n                assert item\n"
            );
            assert_matches_cpython(&source, "protected_nested_conditional.py", &LEVELS);
        }
    }
}

#[test]
fn async_iterator_preserves_erased_branch_backedges() {
    for manager in ["with", "async with"] {
        for iterator in ["for", "async for"] {
            let source = format!(
                "async def collect(manager, items):\n    {manager} manager:\n        with subscribe():\n            previous = None\n            {iterator} item in items:\n                if valid(item) and item.kind == 'entry':\n                    if previous is None:\n                        assert item.value is not None\n                        previous = item.value\n                        assert previous\n                    else:\n                        assert item.value == previous\n            assert previous is not None\n"
            );
            assert_matches_cpython(&source, "protected_iterator_erased_branches.py", &LEVELS);
        }
    }
}

#[test]
fn async_iterator_chained_condition_preserves_erased_else_backedge() {
    for iterator in ["for", "async for"] {
        for condition in [
            "previous == item.value == None",
            "previous is item.value is None",
            "0 < item.value < 3",
            "previous == item.value == None and ready(item)",
            "previous == item.value == None or ready(item)",
            "not (previous == item.value == None)",
            "previous == await read(item) == None",
            "previous == (item.value if ready(item) else fallback()) == None",
        ] {
            let source = format!(
                "async def collect(manager, items):\n    async with manager:\n        with subscribe():\n            previous = None\n            {iterator} item in items:\n                if valid(item) and item.kind == 'entry':\n                    if {condition}:\n                        assert item.value is not None\n                        previous = item.value\n                    else:\n                        assert item.value == previous\n            assert previous is not None\n"
            );
            assert_matches_cpython(&source, "iterator_chained_erased_else.py", &LEVELS);
        }
    }
}

#[test]
fn async_iterator_erased_suffix_preserves_inner_manager_exit() {
    for outer in ["async with manager:", "try:"] {
        let handler = if outer == "try:" {
            "    except Error:\n        recover()\n"
        } else {
            ""
        };
        let source = format!(
            "async def collect(manager, items):\n    {outer}\n        with subscribe():\n            async for item in items:\n                consume(item)\n            assert items\n{handler}"
        );
        assert_matches_cpython(&source, "async_iterator_inner_manager_exit.py", &LEVELS);
    }
}

#[test]
fn erased_nested_branches_preserve_async_iterator_join() {
    let source = "async def f(values, flag):\n    items = []\n    async for value in values:\n        if not items:\n            if flag:\n                assert value\n            else:\n                assert value\n        else:\n            assert value\n        items.append(value)\n";
    for suffix in ["", "    result = tuple(items)\n    assert result\n"] {
        assert_matches_cpython(
            &format!("{source}{suffix}"),
            "async_iterator_elided_join.py",
            &LEVELS,
        );
    }
}

#[test]
fn erased_coroutine_comparison_arms_preserve_shared_cleanup() {
    for prefix in ["", "    await worker()\n"] {
        let source = format!(
            "async def f(a, b, worker):\n{prefix}    if a == b == \"ready\":\n        assert a\n    else:\n        assert b\n"
        );
        assert_matches_cpython(&source, "coroutine_comparison_cleanup.py", &LEVELS);
    }
}

#[test]
fn coroutine_protected_iterator_preserves_erased_else_backedge() {
    for manager in ["with", "async with"] {
        for body in ["previous = value", "consume(value)", "await consume(value)"] {
            let source = format!(
                "async def collect(manager, items):\n    {manager} manager:\n        previous = None\n        for item in items:\n            value = item.value\n            assert value\n            if previous is None:\n                {body}\n            else:\n                assert previous is value\n"
            );
            assert_matches_cpython(&source, "coroutine_protected_iterator_else.py", &LEVELS);
        }
    }
    assert_matches_cpython(
        "async def collect(manager, items):\n    with manager:\n        previous = None\n        for item in items:\n            async def prepare():\n                async with open_stream(item) as stream:\n                    async for packet in stream:\n                        pass\n            with capture():\n                await prepare()\n            value = item.value\n            if previous is None:\n                previous = value\n            else:\n                assert previous is value\n",
        "coroutine_nested_definition_iterator_else.py",
        &LEVELS,
    );
}

#[test]
fn unpacked_store_preserves_loop_backedge_location() {
    let source = "def reset_groups(groups, enabled, limit):\n    first, second, total = 0, 0, 0\n    for group in groups:\n        for item in group:\n            consume(item)\n            if enabled and (item > limit or total > limit):\n                if item:\n                    report(item)\n                first, second, total = 0, 0, 0\n            assert first >= 0\n    finish(total)\n";
    for (index, source) in [
        source.to_owned(),
        source.replace(
            "                if item:\n                    report(item)\n",
            "                report(item)\n",
        ),
        source.replace("first, second, total = 0, 0, 0", "first, second = 0, 0"),
        source.replace(
            "first, second, total = 0, 0, 0",
            "first, second, first = 0, 0, 0",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("unpacked_store_backedge_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_finally_branch_preserves_conditional_continuation() {
    let source = "def configure(kind, ready):\n    try:\n        if kind == 'first':\n            assert ready\n        elif kind == 'second':\n            assert kind\n        else:\n            assert False\n        if ready and available():\n            consume(kind)\n    finally:\n        finish()\n";
    for (index, source) in [
        source.to_owned(),
        source.replace(
            "        elif kind == 'second':\n            assert kind\n",
            "",
        ),
        source.replace(
            "        if ready and available():\n            consume(kind)",
            "        consume(kind)",
        ),
        source.replace(
            "        if ready and available():",
            "        assert ready\n        if ready and available():",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("erased_finally_continuation_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn coroutine_direct_nested_erased_arms_preserve_join() {
    for (filename, source) in [
        (
            "coroutine-both-erased.py",
            r"async def sample(outer, inner, value, consume):
    if outer:
        if inner:
            assert value
        else:
            assert inner
    else:
        assert outer
    result = consume(value)
    consume(result)
    return result
",
        ),
        (
            "ordinary-both-erased.py",
            r"def sample(outer, inner, value, consume):
    if outer:
        if inner:
            assert value
        else:
            assert inner
    else:
        assert outer
    result = consume(value)
    consume(result)
    return result
",
        ),
        (
            "coroutine-mixed-inner.py",
            r"async def sample(outer, inner, value, consume):
    if outer:
        if inner:
            temporary = value
        else:
            assert inner
    else:
        assert outer
    result = consume(value)
    consume(result)
    return result
",
        ),
        (
            "coroutine-live-inner.py",
            r"async def sample(outer, inner, value, consume):
    if outer:
        if inner:
            temporary = value
        else:
            temporary = inner
    else:
        assert outer
    result = consume(value)
    consume(result)
    return result
",
        ),
        (
            "coroutine-no-outer-else.py",
            r"async def sample(outer, inner, value, consume):
    if outer:
        if inner:
            assert value
        else:
            assert inner
    result = consume(value)
    consume(result)
    return result
",
        ),
        (
            "coroutine-flat-erased.py",
            r"async def sample(outer, inner, value, consume):
    if inner:
        assert value
    else:
        assert inner
    result = consume(value)
    consume(result)
    return result
",
        ),
    ] {
        assert_matches_cpython(source, filename, &LEVELS);
    }
}

#[test]
fn coroutine_nested_erased_arms_with_surrounding_work_preserve_join() {
    for (filename, source) in [
        (
            "erased-prefix.py",
            r"async def inspect_path(primary, secondary, packet, emit):
    if primary:
        assert packet
        if secondary:
            assert secondary
        else:
            assert primary
    else:
        assert packet
    outcome = emit(packet)
    emit(outcome)
    return outcome
",
        ),
        (
            "erased-suffix.py",
            r"async def inspect_path(primary, secondary, packet, emit):
    if primary:
        if secondary:
            assert secondary
        else:
            assert primary
        assert packet
    else:
        assert packet
    outcome = emit(packet)
    emit(outcome)
    return outcome
",
        ),
        (
            "erased-prefix-and-suffix.py",
            r"async def inspect_path(primary, secondary, packet, emit):
    if primary:
        assert packet
        if secondary:
            assert secondary
        else:
            assert primary
        assert packet
    else:
        assert packet
    outcome = emit(packet)
    emit(outcome)
    return outcome
",
        ),
        (
            "linear-prefix.py",
            r"async def inspect_path(primary, secondary, packet, emit):
    if primary:
        observed = packet
        if secondary:
            assert observed
        else:
            assert primary
    else:
        assert packet
    outcome = emit(packet)
    emit(outcome)
    return outcome
",
        ),
    ] {
        assert_matches_cpython(source, filename, &LEVELS);
    }
}

#[test]
fn coroutine_nested_mixed_erased_arms_preserve_join() {
    for (filename, source) in [
        (
            "mixed-inner-control.py",
            r"async def inspect_path(primary, secondary, packet, emit):
    if primary:
        assert packet
        if secondary:
            assert secondary
        else:
            emit(packet)
    else:
        assert packet
    outcome = emit(packet)
    emit(outcome)
    return outcome
",
        ),
        (
            "mixed-inner-mirror.py",
            r"async def inspect_path(primary, secondary, packet, emit):
    if primary:
        assert packet
        if secondary:
            emit(packet)
        else:
            assert primary
    else:
        assert packet
    outcome = emit(packet)
    emit(outcome)
    return outcome
",
        ),
    ] {
        assert_matches_cpython(source, filename, &LEVELS);
    }
}

#[test]
fn coroutine_nested_erased_arms_after_completed_await_preserve_join() {
    for (filename, source) in [
        (
            "standalone-await-prefix.py",
            r"async def inspect_path(primary, secondary, packet, emit):
    if primary:
        await emit(packet)
        if secondary:
            assert secondary
        else:
            assert primary
    else:
        assert packet
    outcome = emit(packet)
    emit(outcome)
    return outcome
",
        ),
        (
            "no-await-control.py",
            r"async def inspect_path(primary, secondary, packet, emit):
    if primary:
        assert packet
        if secondary:
            assert secondary
        else:
            assert primary
    else:
        assert packet
    outcome = emit(packet)
    emit(outcome)
    return outcome
",
        ),
        (
            "await-outside-outer.py",
            r"async def inspect_path(primary, secondary, packet, emit):
    await emit(packet)
    if primary:
        assert packet
        if secondary:
            assert secondary
        else:
            assert primary
    else:
        assert packet
    outcome = emit(packet)
    emit(outcome)
    return outcome
",
        ),
        (
            "await-inner-arm.py",
            r"async def inspect_path(primary, secondary, packet, emit):
    if primary:
        assert packet
        if secondary:
            await emit(packet)
        else:
            assert primary
    else:
        assert packet
    outcome = emit(packet)
    emit(outcome)
    return outcome
",
        ),
    ] {
        assert_matches_cpython(source, filename, &LEVELS);
    }

    // These eight optimized modern residuals remain in the paired raw-byte screen.
    // Together with the cases above, the unaffected cells make 64 equality checks.
    for (filename, source) in [
        (
            "assigned-await-prefix.py",
            r"async def inspect_path(primary, secondary, packet, emit):
    if primary:
        observed = await emit(packet)
        if secondary:
            assert secondary
        else:
            assert primary
    else:
        assert packet
    outcome = emit(packet)
    emit(outcome)
    return outcome
",
        ),
        (
            "await-inner-test.py",
            r"async def inspect_path(primary, secondary, packet, emit):
    if primary:
        assert packet
        if await emit(secondary):
            assert secondary
        else:
            assert primary
    else:
        assert packet
    outcome = emit(packet)
    emit(outcome)
    return outcome
",
        ),
    ] {
        assert_matches_cpython_versions(
            source,
            filename,
            &LEVELS,
            &[PythonVersion::Py312, PythonVersion::Py313],
        );
        assert_matches_cpython_versions(
            source,
            filename,
            &[OptimizationLevel::Zero],
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn coroutine_completed_await_inner_tests_preserve_erased_join() {
    for (filename, source) in [
        (
            "inner-await-call.py",
            r"async def inspect_path(primary, secondary, packet, emit):
    if primary:
        assert packet
        if await emit(secondary):
            assert secondary
        else:
            assert primary
    else:
        assert packet
    outcome = emit(packet)
    emit(outcome)
    return outcome
",
        ),
        (
            "inner-await-multiline-call.py",
            r"async def inspect_path(primary, secondary, packet, emit):
    if primary:
        assert packet
        if await emit(
            secondary
        ):
            assert secondary
        else:
            assert primary
    else:
        assert packet
    outcome = emit(packet)
    emit(outcome)
    return outcome
",
        ),
        (
            "inner-await-multiline-await.py",
            r"async def inspect_path(primary, secondary, packet, emit):
    if primary:
        assert packet
        if (
            await emit(secondary)
        ):
            assert secondary
        else:
            assert primary
    else:
        assert packet
    outcome = emit(packet)
    emit(outcome)
    return outcome
",
        ),
    ] {
        assert_matches_cpython(source, filename, &LEVELS);
    }
}

#[test]
fn ordinary_try_nested_erased_arms_preserve_tuple_return_join() {
    for (filename, source) in [
        (
            "direct-short.py",
            r"def sample(outer, inner, a, b, c, operation):
    if outer:
        operation(a)
    else:
        assert b
    return a, b
",
        ),
        (
            "direct-long.py",
            r"def sample(outer, inner, a, b, c, operation):
    if outer:
        operation(a)
    else:
        assert b
    return a, b, c
",
        ),
        (
            "nested-short.py",
            r"def sample(outer, inner, a, b, c, operation):
    if outer:
        if inner:
            assert a
        else:
            assert b
    else:
        assert c
    return a, b
",
        ),
        (
            "nested-long.py",
            r"def sample(outer, inner, a, b, c, operation):
    if outer:
        if inner:
            assert a
        else:
            assert b
    else:
        assert c
    return a, b, c
",
        ),
        (
            "protected-short.py",
            r"def sample(outer, inner, a, b, c, operation):
    try:
        if outer:
            if inner:
                assert a
            else:
                assert b
        else:
            assert c
        return a, b
    except RuntimeError:
        return None
",
        ),
        (
            "protected-long.py",
            r"def sample(outer, inner, a, b, c, operation):
    try:
        if outer:
            if inner:
                assert a
            else:
                assert b
        else:
            assert c
        return a, b, c
    except RuntimeError:
        return None
",
        ),
    ] {
        assert_matches_cpython_versions(
            source,
            filename,
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn coroutine_populated_nested_branch_preserves_erased_join() {
    for condition in ["isinstance(item, dict)", "accept(item)"] {
        let source = format!(
            "async def normalize(record):\n    await prepare()\n    if record.values is not None:\n        assert record.values\n        item = record.values[0]\n        if {condition}:\n            label = item.get(\"label\")\n        else:\n            label = getattr(item, \"label\", None)\n        assert label\n    else:\n        assert record.values is None\n    await finish()\n"
        );
        assert_matches_cpython(&source, "coroutine_populated_nested_join.py", &LEVELS);
    }
}

#[test]
fn erased_assertion_preserves_legacy_loop_return_copies() {
    let source = "def f(worker):\n    if not worker.ready:\n        value = worker.read()\n        while value not in [\"a\", \"b\"]:\n            value = worker.read()\n        assert value == \"a\"\n";
    for (index, source) in [
        source.to_owned(),
        source.replace(
            "    if not worker.ready:\n",
            "    if worker.invalid:\n        raise ValueError(\"invalid\")\n    if not worker.ready:\n",
        ),
        source.replace("        assert value == \"a\"\n", "        pass\n"),
        format!("{source}        worker.finish()\n"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("legacy_loop_return_{index}.py"), &LEVELS);
    }
}

#[test]
fn erased_loop_branch_before_condition_keeps_protected_join() {
    for condition in ["first", "first and third", "first or third", "not first"] {
        for gap in ["", "                assert third\n"] {
            let source = format!(
                "def f(manager, items, first, second, third):\n    try:\n        with manager:\n            for item in items:\n                if {condition}:\n                    assert first\n                else:\n                    assert second\n{gap}                if second:\n                    assert second\n                else:\n                    assert first\n    except Exception:\n        recover()\n"
            );
            assert_matches_cpython(&source, "erased_loop_condition_join.py", &LEVELS);
        }
    }
    assert_matches_cpython(
        "def f(manager, items, first, second):\n    with manager:\n        try:\n            for item in items:\n                if first:\n                    assert first\n                else:\n                    assert second\n                if second:\n                    assert second\n                else:\n                    assert first\n        except Exception:\n            recover()\n",
        "erased_loop_condition_inner_try.py",
        &LEVELS,
    );
}

#[test]
fn enclosing_try_else_preserves_manager_borrow_boundary() {
    for (scope, manager, work) in [
        ("def", "with", "consume(value)"),
        ("async def", "with", "consume(value)"),
        ("async def", "with", "await consume(value)"),
        ("async def", "async with", "await consume(value)"),
    ] {
        for handler in ["Exception", "Exception as error"] {
            for otherwise in ["consume(result)", "outcome = 1 if result else 2"] {
                for cleanup in [
                    "cleanup(value)\n",
                    "if value is not None:\n    cleanup(value)\n",
                ] {
                    let mut source = format!(
                        "{scope} f(result, value, flag):\n    try:\n        try:\n            {manager} manager():\n                result = {work}\n        except {handler}:\n            raise\n    except Exception:\n        raise\n    else:\n        {otherwise}\n    finally:\n"
                    );
                    for line in cleanup.lines() {
                        writeln!(source, "        {line}").unwrap();
                    }
                    assert_matches_cpython(&source, "enclosing_manager_try_else.py", &LEVELS);
                }
            }
        }
    }
    for scope in ["def", "async def"] {
        let source = format!(
            "{scope} f(result, value, flag):\n    try:\n        try:\n            with manager():\n                result = consume(value)\n        except Exception:\n            raise\n    except Exception:\n        raise\n    else:\n        consume(result)\n    finally:\n        cleanup(value)\n"
        );
        for source in [
            source.replace("    finally:\n        cleanup(value)\n", ""),
            source.replace(
                "            raise\n",
                "            raise\n        else:\n            consume(flag)\n",
            ),
            source.replace(
                "            raise\n",
                "            raise\n        finally:\n            cleanup(flag)\n",
            ),
            source.replace(
                "            raise\n",
                "            raise\n        consume(flag)\n",
            ),
        ] {
            assert_matches_cpython(&source, "enclosing_manager_try_else_boundary.py", &LEVELS);
        }
    }
}

#[test]
fn erased_try_else_preserves_manager_borrow_boundary() {
    let source = "async def run():\n    try:\n        async with manager() as resource:\n            await resource.first()\n    except RuntimeError as error:\n        assert error\n    else:\n        assert False\n    async with manager() as resource:\n        await resource.second()\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("    else:\n        assert False\n", ""),
        source.replace("        assert False\n", "        pass\n"),
        source.replace("        assert False\n", "        finish()\n"),
        source.replace("        async with manager()", "        with manager()"),
        source.replace(
            "        assert False\n",
            "        assert False\n    finally:\n        finish()\n",
        ),
        "async def run():\n    async with manager() as resource:\n        pass\n    try:\n        await resource.first()\n    except ValueError:\n        pass\n    else:\n        assert False\n    configure()\n    async with manager() as resource:\n        await resource.second()\n".to_owned(),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("erased_manager_try_else_{index}.py"), &LEVELS);
    }
}

#[test]
fn erased_suffix_preserves_with_pop_block() {
    let source = "def check(manager):\n    with manager:\n        try:\n            action()\n        except OSError as error:\n            assert error\n        else:\n            raise AssertionError()\n        assert manager\n        assert manager\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("        assert manager\n", ""),
        source.replace("        assert manager\n        assert manager\n", "        finish()\n"),
        source.replace("            assert error\n", "            pass\n"),
        source.replace("except OSError as error:", "except OSError:").replace("            assert error\n", "            assert manager\n"),
        "def check(manager, items):\n    with manager:\n        for item in items:\n            if item:\n                break\n        else:\n            raise ValueError()\n        assert items\n".to_owned(),
        "def check(manager, ready):\n    with manager:\n        while ready():\n            if stop():\n                break\n        else:\n            raise ValueError()\n        assert ready\n".to_owned(),
        "def check(first, second):\n    with first:\n        with second:\n            raise ValueError()\n        assert first\n".to_owned(),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("erased_with_tail_{index}.py"), &LEVELS);
    }
}

#[test]
fn erased_coroutine_handler_assertions_preserve_manager_pop_block() {
    for work in ["worker()", "await worker()"] {
        for exit in ["raise Failure('expected')", "return 42"] {
            let source = format!(
                "async def verify(manager, worker):\n    with manager:\n        try:\n            {work}\n            {exit}\n        except Problem as error:\n            assert error.value\n    await worker()\n"
            );
            assert_matches_cpython(&source, "erased_coroutine_handler_exit.py", &LEVELS);
        }
    }
}

#[test]
fn erased_module_handler_assertions_preserve_manager_pop_block() {
    let source = "with manager:\n    try:\n        raise ValueError\n    except ValueError as problem:\n        assert ready\n";
    assert_matches_cpython_versions(
        source,
        "erased_module_handler_exit.py",
        &LEVELS,
        &[PythonVersion::Py314, PythonVersion::Py315],
    );
}

#[test]
fn erased_handler_assertions_preserve_manager_pop_block() {
    const SOURCE: &str = "def f(manager, factory):\n    with manager():\n        obj = factory()\n        try:\n            obj.run()\n            raise ValueError(\"failed\")\n        except Error as exc:\n            text = str(exc)\n            assert \"bad\" in text\n        assert obj.ready\n    finish()\n";
    for (index, assignment) in [
        "text = str(exc)",
        "obj.text = str(exc)",
        "text, other = unpack(exc)",
        "text, *other = unpack(exc)",
        "text = obj.text = str(exc)",
        "(obj\n             .text) = str(exc)",
    ]
    .into_iter()
    .enumerate()
    {
        let source = SOURCE.replace("text = str(exc)", assignment);
        assert_matches_cpython(
            &source,
            &format!("erased_handler_store_{index}.py"),
            &LEVELS,
        );
        let source = source
            .replace("            assert \"bad\" in text\n", "")
            .replace("        assert obj.ready\n", "");
        assert_matches_cpython(&source, &format!("live_handler_store_{index}.py"), &LEVELS);
    }
    for (index, source) in [
        SOURCE.replace("        assert obj.ready\n", ""),
        SOURCE.replace("            assert \"bad\" in text\n", ""),
        SOURCE
            .replace("except Error as exc:", "except Error:")
            .replace("str(exc)", "str(obj)"),
        SOURCE.replace("text = str(exc)", "consume(exc)"),
        SOURCE.replace(
            "        assert obj.ready\n",
            "        assert obj.ready\n        obj.finish()\n",
        ),
        SOURCE.replace("            raise ValueError(\"failed\")\n", ""),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("handler_manager_boundary_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_assertions_preserve_manager_while_exit() {
    let source = "def f(manager, worker):\n    with manager:\n        while len(worker.items) != 1:\n            worker.advance()\n        assert len(worker.items) == 1\n";
    let repeated = source.replace(
        "        assert len(worker.items) == 1\n",
        "        assert len(worker.items) == 1\n        worker.restart()\n        while len(worker.items) != 2:\n            worker.advance()\n        assert len(worker.items) == 2\n",
    );
    for (index, source) in [
        source.to_owned(),
        repeated,
        source.replace("        assert len(worker.items) == 1\n", "        pass\n"),
        format!("{source}        worker.finish()\n"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("manager_while_exit_{index}.py"), &LEVELS);
    }
}

#[test]
fn elided_assertion_preserves_conditional_manager_store_location() {
    for assignment in ["value: object = await work()", "value = await work()"] {
        for suffix in ["assert valid(value)", "pass", "check(value)"] {
            let source = format!(
                "async def f(manager, flag):\n    with manager:\n        if flag:\n            {assignment}\n            return\n        else:\n            {assignment}\n            {suffix}\n    after()\n"
            );
            assert_matches_cpython(&source, "conditional_manager_store.py", &LEVELS);
        }
    }
}

#[test]
fn comprehension_assignment_prefix_preserves_finalizer_manager_exit() {
    for suffix in ["assert valid(values)", "pass", "check(values)"] {
        let source = format!(
            "def f(manager, items):\n    with manager:\n        values = [work(item) for item in items]\n        try:\n            assert ready()\n            wait()\n        finally:\n            finish()\n        {suffix}\n"
        );
        assert_matches_cpython(&source, "comprehension_prefix_finalizer.py", &LEVELS);
    }
}

#[test]
fn completed_finalizer_prefix_preserves_manager_exit() {
    for suffix in ["assert done(second)", "pass", "check(second)"] {
        let source = format!(
            "def f(first, second):\n    with first, second:\n        value = acquire(first)\n        try:\n            other = acquire(second)\n        finally:\n            release(first)\n        assert value\n        assert other\n        value = acquire(second)\n        try:\n            assert value\n        finally:\n            release(second)\n        {suffix}\n"
        );
        assert_matches_cpython(&source, "completed_finalizer_prefix.py", &LEVELS);
    }
}

#[test]
fn completed_iterator_prefix_preserves_finalizer_manager_exit() {
    for prefix in [
        "        for item in items:\n            begin(item)\n        value = []\n",
        "        items = [make(item) for item in items]\n        for item in items:\n            begin(item)\n        value = []\n",
    ] {
        for suffix in ["assert valid(items)", "pass", "check(items)"] {
            let source = format!(
                "def f(manager, items):\n    with manager:\n{prefix}        try:\n            for item in items:\n                work(item)\n        except Error as error:\n            raise Failure() from error\n        finally:\n            for item in items:\n                finish(item)\n            for item in items:\n                if active(item):\n                    stop(item)\n                    finish(item)\n        {suffix}\n    after()\n"
            );
            assert_matches_cpython(&source, "iterator_prefix_finalizer.py", &LEVELS);
        }
    }
}

#[test]
fn elided_conditional_preserves_jump_before_finalizer() {
    for body in ["assert value.ready", "pass", "record(value)"] {
        for successor in [
            "",
            "        record(value)\n",
            "        result = build(value)\n",
        ] {
            let source = format!(
                "def f(flag):\n    try:\n        value = work()\n        assert value\n        if flag:\n            {body}\n        else:\n            assert value.done\n{successor}    finally:\n        cleanup()\n        finish()\n"
            );
            assert_matches_cpython(&source, "conditional_before_finalizer.py", &LEVELS);
        }
    }
}

#[test]
fn elided_conditional_preserves_jump_before_coroutine_finalizer() {
    for cleanup in ["cleanup()", "await cleanup()"] {
        for body in ["assert value.ready", "pass", "record(value)"] {
            let source = format!(
                "async def f(flag):\n    try:\n        value = await work()\n        if flag:\n            {body}\n        else:\n            assert value.done\n    finally:\n        {cleanup}\n"
            );
            assert_matches_cpython(&source, "conditional_coroutine_finalizer.py", &LEVELS);
        }
    }
}

#[test]
fn elided_conditional_preserves_jump_before_erased_try_tail() {
    for prefix in [
        "        value = work()\n",
        "        with manager:\n            value = work()\n",
    ] {
        for suffix in ["assert complete()", "record()"] {
            let source = format!(
                "async def f(manager, flag):\n    try:\n{prefix}        if flag == expected:\n            assert valid(value)\n        else:\n            assert ready(value)\n        assert done()\n        {suffix}\n    finally:\n        cleanup()\n        finish()\n"
            );
            assert_matches_cpython(&source, "conditional_erased_try_tail.py", &LEVELS);
        }
    }
}

#[test]
fn coroutine_manager_prefix_preserves_protected_finalizer_join() {
    let indent = |source: &str| {
        let mut result = String::new();
        for line in source.lines() {
            writeln!(result, "    {line}").unwrap();
        }
        result
    };
    for managers in [
        &[][..],
        &["with first:"],
        &["async with first:"],
        &["with first:", "with second:"],
        &["async with first:", "async with second:"],
        &["with first:", "async with second:"],
        &["async with first:", "with second:"],
    ] {
        for finalizer in [
            "callback()\n",
            "await callback()\n",
            "await value\n",
            "if value:\n    callback()\n",
        ] {
            for test in ["flag", "flag and value"] {
                let mut body = format!(
                    "try:\n    if {test}:\n        assert value\n    else:\n        assert not value\nfinally:\n{}",
                    indent(finalizer)
                );
                for manager in managers.iter().rev() {
                    body = format!("{manager}\n{}", indent(&body));
                }
                let source = format!(
                    "async def sample(flag, value, first, second, callback):\n{}",
                    indent(&body)
                );
                assert_matches_cpython(&source, "coroutine_manager_prefix.py", &LEVELS);
            }
        }
    }
}

#[test]
fn elided_conditional_preserves_jump_before_managed_finalizer() {
    for managers in ["manager", "manager, other"] {
        for body in ["assert value.ready", "pass", "record(value)"] {
            let source = format!(
                "def f(manager, other, flag):\n    with {managers}:\n        value: object = setup()\n        try:\n            work(value)\n            if flag is None:\n                {body}\n            else:\n                assert value.done\n        finally:\n            cleanup(value)\n"
            );
            assert_matches_cpython(&source, "conditional_managed_finalizer.py", &LEVELS);
        }
    }
}

#[test]
fn elided_conditional_preserves_jump_before_guarded_finalizer() {
    for asynchronous in [false, true] {
        for body in ["assert valid()", "pass", "record()"] {
            let source = format!(
                "async def f(flag, resource):\n    try:\n        await work()\n        if flag:\n            {body}\n        else:\n            assert ready()\n    finally:\n        if resource is not None:\n            resource.close()\n            await finish(resource)\n"
            );
            let source = if asynchronous {
                source
            } else {
                source.replace("async def", "def").replace("await ", "")
            };
            assert_matches_cpython(&source, "conditional_guarded_finalizer.py", &LEVELS);
        }
    }
}

#[test]
fn elided_conditional_preserves_jump_before_nested_finalizers() {
    for managers in ["manager", "manager, other"] {
        for body in ["assert valid()", "pass", "record()"] {
            let source = format!(
                "def f(manager, other, flag, resource):\n    try:\n        with {managers}:\n            begin()\n            try:\n                work()\n                if flag:\n                    {body}\n                else:\n                    assert ready()\n            finally:\n                if resource is not None:\n                    resource.close()\n                finish()\n    finally:\n        cleanup()\n"
            );
            assert_matches_cpython(&source, "conditional_nested_finalizers.py", &LEVELS);
        }
    }
}

#[test]
fn elided_handler_assertion_preserves_shared_false_cleanup() {
    for (condition, suffix) in [
        ("config.recover and not stopped", "assert False"),
        ("config.recover and not stopped", "pass"),
        ("config.recover and not stopped", "repair(error)"),
        ("config.recover or not stopped", "assert False"),
    ] {
        let source = format!(
            "def collect(items, config, fallback, stopped):\n    for item in items:\n        try:\n            value = parse(item)\n        except ValueError as error:\n            if {condition}:\n                report(error)\n                return fallback\n            {suffix}\n        consume(value)\n    return items\n"
        );
        assert_matches_cpython(&source, "shared_handler_false_cleanup.py", &LEVELS);
    }
}

#[test]
fn elided_handler_assertion_preserves_conditional_cleanup() {
    for suffix in ["assert value in str(error)", "pass", "after()"] {
        let source = format!(
            "async def f(manager, other, value):\n    async with manager:\n        try:\n            async with await other():\n                work()\n        except Error as error:\n            if value in str(error):\n                recover()\n            {suffix}\n"
        );
        assert_matches_cpython(&source, "handler_conditional_cleanup.py", &LEVELS);
    }
}

#[test]
fn elided_handler_assertion_preserves_while_break_cleanup() {
    for suffix in ["assert found", "pass", "after()"] {
        let source = format!(
            "async def f(worker):\n    try:\n        await worker.run()\n    except Error as error:\n        found = False\n        while error.cause:\n            if \"marker\" in str(error):\n                found = True\n                break\n            error = error.cause\n        {suffix}\n"
        );
        assert_matches_cpython(&source, "handler_while_break_cleanup.py", &LEVELS);
    }
    for definition in ["def", "async def"] {
        let source = format!(
            "{definition} f(worker):\n    try:\n        worker.run()\n    except Error as error:\n        cursor = error\n        found = False\n        while cursor.cause:\n            if \"marker\" in cursor.payload.data.decode():\n                found = True\n                break\n            cursor = cursor.cause\n        assert found\n"
        );
        assert_matches_cpython(&source, "handler_cursor_break_cleanup.py", &LEVELS);
    }
}

#[test]
fn erased_assertions_preserve_nested_recovery_borrow_boundary() {
    for definition in ["def", "async def"] {
        for body in [
            "                assert obj.state is None\n                obj.state = {}\n",
            "                obj.state = {}\n                assert obj.state is None\n",
            "                obj.state = {}\n",
        ] {
            let source = format!(
                "{definition} f(obj, flag):\n    try:\n        if flag and ready(obj):\n            assert obj.ready\n            try:\n{body}            except Exception:\n                pass\n    except Exception:\n        return None\n    use(obj)\n    use(obj)\n"
            );
            assert_matches_cpython(&source, "nested_recovery_borrow.py", &LEVELS);
        }
    }
}

#[test]
fn erased_nested_else_preserves_try_exit_borrow_boundary() {
    let source = "def validate(values, enabled):\n    item = values[0]\n    try:\n        if enabled:\n            if valid(item):\n                consume(item)\n            else:\n                assert item\n    except Exception:\n        return False\n    if 'payload' in item:\n        assert item.payload\n    return True\n";
    let preceding_branch = source.replace(
        "    try:\n",
        "    try:\n        prepare(item)\n        if valid(item):\n            consume(item)\n        else:\n            assert item\n",
    );
    for (index, source) in [
        source.to_owned(),
        preceding_branch.replace(
            "    except Exception:\n",
            "    except Exception as error:\n        report(error)\n",
        ),
        source.replace("                assert item\n", "                consume(item)\n"),
        source.replace("                assert item\n", "                pass\n"),
        source.replace(
            "            else:\n",
            "            elif alternate(item):\n                consume(item)\n            else:\n",
        ),
        source.replace(
            "        if enabled:\n            if valid(item):\n                consume(item)\n            else:\n                assert item\n",
            "        if valid(item):\n            consume(item)\n        else:\n            assert item\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("nested_erased_else_try_exit_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_conditional_preserves_join_before_async_manager_exit() {
    for body in ["assert flag", "pass"] {
        let source = format!(
            "async def f(manager, outer, inner, stop, flag, work):\n    with manager:\n        while not stop:\n            async with outer:\n                async with inner:\n                    if not flag:\n                        {body}\n                    else:\n                        {body}\n            await work()\n"
        );
        assert_matches_cpython(&source, "async_manager_conditional_join.py", &LEVELS);
    }
    for source in [
        "async def f(inner, stop, flag, work):\n    while not stop:\n        async with inner:\n            if not flag:\n                assert flag\n            else:\n                assert flag\n        await work()\n",
        "async def f(outer, inner, flag, work):\n    async with outer:\n        async with inner:\n            if not flag:\n                assert flag\n            else:\n                assert flag\n    await work()\n",
    ] {
        assert_matches_cpython(source, "async_manager_loop_boundary.py", &LEVELS);
    }
}

#[test]
fn conditional_call_preserves_erased_else_try_exit() {
    for (index, (call, otherwise)) in [
        ("consume(item if enabled else values)", "assert item"),
        (
            "consume(\n                    value=item if enabled else values,\n                )",
            "assert item",
        ),
        ("(consume if enabled else report)(item)", "assert item"),
        (
            "if enabled:\n                    prepare(item)\n                else:\n                    prepare(values)\n                consume(item if enabled else values)",
            "assert item",
        ),
        ("consume(item) if enabled else consume(values)", "assert item"),
        ("consume(item if enabled else values)", "consume(item)"),
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!(
            "def validate(values, enabled):\n    item = values[0]\n    try:\n        if enabled:\n            if valid(item):\n                {call}\n            else:\n                {otherwise}\n    except Exception:\n        return False\n    if 'payload' in item:\n        assert item.payload\n    return True\n"
        );
        assert_matches_cpython(
            &source,
            &format!("conditional_call_try_exit_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_assertion_preserves_try_exit_borrow_boundary() {
    for definition in ["def", "async def"] {
        for suffix in ["assert value.kind == 'item'", "pass", "check(value)"] {
            let source = format!(
                "{definition} collect(items):\n    result = []\n    for entry in items:\n        try:\n            value = entry.read().value\n            if value.kind == 'group':\n                assert len(value.parts) == 1\n                value = value.parts[0]\n            {suffix}\n        except Error as error:\n            report(error)\n            value = fallback()\n        result.append(value)\n    return result\n"
            );
            assert_matches_cpython(&source, "erased_try_exit_assertion.py", &LEVELS);
        }
    }
    assert_matches_cpython(
        "def collect(items):\n    result = []\n    for entry in items:\n        try:\n            value = entry.read()\n            while value.ready and value.active:\n                value = step(value)\n            assert value\n        except Error:\n            value = fallback()\n        result.append(value)\n    return result\n",
        "erased_while_try_exit_assertion.py",
        &LEVELS,
    );
}

#[test]
fn elided_assertion_after_nested_iterator_preserves_condition_backedges() {
    for suffix in ["assert item", "pass", "after()"] {
        let source = format!(
            "def f(items, first, second, left, right, target):\n    for item in items:\n        if first:\n            if left and right:\n                for value in item:\n                    del target[value]\n            {suffix}\n        elif second:\n            values = [value for value in item]\n            assert values\n        else:\n            assert item\n"
        );
        assert_matches_cpython(&source, "nested_iterator_condition_backedges.py", &LEVELS);
    }
    assert_matches_cpython(
        "def f(items, first, left, right, target):\n    for item in items:\n        if first:\n            if left and right:\n                for value in item:\n                    del target[value]\n            assert item\n",
        "nested_iterator_terminal_condition.py",
        &LEVELS,
    );
}

#[test]
fn coroutine_erased_while_else_normalizes_protected_backedges() {
    let source = "async def f(receive, process):\n    try:\n        while True:\n            value = receive()\n            if isinstance(value, (list, tuple)):\n                first, second, third = value\n                assert third is not None\n                await process(first=first, second=second, third=third)\n            else:\n                assert False, value\n    except ValueError:\n        report()\n";
    for source in [
        source.to_owned(),
        source.replace("while True:", "while receive.enabled:"),
        source.replace("first, second, third = value", "third = value"),
        source.replace("await process(", "result = await process("),
        source.replace("await ", ""),
        source.replace("assert False, value", "pass"),
    ] {
        assert_matches_cpython(&source, "coroutine_protected_while_else.py", &LEVELS);
    }
}

#[test]
fn erased_while_else_normalizes_protected_backedges() {
    let source = "def process(state):\n    with guard():\n        while True:\n            value = receive()\n            if value == \"start\":\n                work()\n            elif value == \"stop\":\n                other()\n            else:\n                assert False, value\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("while True:", "while state.ready:"),
        source.replace("                assert False, value", "                pass"),
        source.replace("                assert False, value", "                recover(value)"),
        source.replace("            else:\n                assert False, value\n", ""),
        source.replace(
            "                other()",
            "                with state.lock:\n                    state.ready = True\n                    state.lock.notify()\n                    while state.ready:\n                        state.lock.wait()\n                publish(None)",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("protected_while_backedge_{index}.py"), &LEVELS);
    }
}

#[test]
fn erased_protected_loop_arms_preserve_condition_polarity() {
    for condition in ["check(item)", "not check(item)"] {
        for body in [
            "assert item",
            "assert item\n                assert item.flag",
            "consume(item)",
        ] {
            let source = format!(
                "def f(items):\n    try:\n        for item in items:\n            if {condition}:\n                {body}\n            else:\n                assert item\n        return True\n    except Exception:\n        return False\n"
            );
            assert_matches_cpython(&source, "erased_protected_loop_arms.py", &LEVELS);
        }
    }
}

#[test]
fn erased_match_capture_body_drops_redundant_success_nop() {
    let source = "def f(items):\n    result = {}\n    for key, value in items:\n        match value:\n            case First(value=a):\n                output = transform(a)\n            case Second(value=b):\n                output = transform(b)\n            case other:\n                assert False, other\n        result[key] = consume(output)\n    return result\n";
    for source in [
        source.to_string(),
        source.replace("assert False, other", "pass"),
        source.replace("assert False, other", "output = transform(other)"),
        source.replace(
            "assert False, other",
            "assert other\n                assert valid(other)",
        ),
        source.replace("case other:", "case other if check(other):"),
        source.replace("    result = {}", "    global other\n    result = {}"),
    ] {
        assert_matches_cpython(&source, "erased_match_capture.py", &LEVELS);
    }
}

#[test]
fn erased_match_cases_keep_unlocated_loop_exits() {
    let source = r"def f(items):
    for item in items:
        match item:
            case First() | Second():
                item.value = 1
            case Third():
                assert item.value is not None
            case Fourth():
                pass
            case _:
                raise ValueError(item)
";
    for source in [
        source.to_owned(),
        source.replace("Third()", "Third() | Fifth()"),
        source.replace("Third()", "Third(value)"),
        source.replace("Third()", "3"),
        source.replace("Third():", "Third() if valid(item):"),
        source.replace("assert item.value is not None", "pass"),
        source.replace("assert item.value is not None", "work(item)"),
        format!("{source}        after(item)\n"),
    ] {
        assert_matches_cpython(&source, "erased_match_loop_exit.py", &LEVELS);
    }
}

#[test]
fn erased_match_cases_preserve_loop_backedges() {
    for body in [
        "assert item",
        "pass",
        "consume(item)",
        "assert item\n                consume(item)",
    ] {
        for suffix in ["", "        after()\n"] {
            let source = format!(
                "def f(items):\n    for item in items:\n        match item:\n            case 1:\n                {body}\n            case 2:\n                {body}\n            case _:\n                assert False\n{suffix}"
            );
            assert_matches_cpython(&source, "erased_match_loop_backedges.py", &LEVELS);
        }
    }
}

#[test]
fn elided_async_loop_branches_keep_augmented_assignment_join() {
    for (index, (function, loop_kind, successor)) in [
        ("async def", "async for", "count += 1"),
        ("async def", "async for", "count = count + 1"),
        ("def", "for", "count += 1"),
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!(
            "{function} count_events(items):\n    count = 0\n    {loop_kind} item in items:\n        if count == 0:\n            assert item\n        elif count == 1:\n            assert item\n        else:\n            assert item\n        {successor}\n    assert count == 3\n"
        );
        assert_matches_cpython(
            &source,
            &format!("elided_async_loop_join_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn coroutine_raise_preserves_repeated_scope_footer() {
    for prefix in [
        "    if expected is not None:\n        assert value == expected\n",
        "    if expected is not None:\n        pass\n",
        "",
    ] {
        let source = format!(
            "async def f(expected, error, calls, value):\n{prefix}    calls.append(value)\n    if error is not None:\n        raise error\n"
        );
        assert_matches_cpython(&source, "coroutine_raise_footer.py", &LEVELS);
    }
    assert_matches_cpython(
        "def outer(expected, error, calls):\n    async def f(value):\n        if expected is not None:\n            assert value == expected\n        calls.append(value)\n        if error is not None:\n            raise error\n    return f\n",
        "coroutine_closure_raise_footer.py",
        &LEVELS,
    );
}

#[test]
fn elided_finally_assertion_preserves_exceptional_exit_locations() {
    for tail in ["assert flag", "pass", "after()"] {
        let source = format!(
            "def f(manager, flag):\n    with manager:\n        try:\n            work()\n        finally:\n            if flag:\n                cleanup()\n            {tail}\n"
        );
        assert_matches_cpython(&source, "finally_assertion_tail.py", &LEVELS);
    }
    assert_matches_cpython(
        "def f(manager, flag):\n    with manager:\n        try:\n            work()\n        finally:\n            while flag:\n                cleanup()\n            assert flag\n",
        "finally_loop_assertion_tail.py",
        &LEVELS,
    );
}

#[test]
fn elided_finally_assertion_preserves_assignment_exit_locations() {
    let source = "def f(items):\n    try:\n        yield\n    finally:\n        values = {\n            key: value\n            for key, value in items\n        }\n        assert values\n";
    for source in [
        source.to_owned(),
        source.replace("values =", "values: dict ="),
        source.replace("yield", "work()"),
        source
            .replace("def f(", "async def f(")
            .replace("yield", "await work()"),
        source.replace("        assert values\n", ""),
        format!("{source}        pass\n"),
        format!("{source}        work()\n"),
        format!("{source}        global flag\n"),
    ] {
        assert_matches_cpython(&source, "finally_assignment_assertion_tail.py", &LEVELS);
    }
}

#[test]
fn folded_comprehension_filter_chains_preserve_borrow_boundaries() {
    for (index, filter) in [
        "not __debug__ and mapping.get(item) and item",
        "mapping.get(item) and not __debug__ and item",
        "mapping.get(item) and item and not __debug__",
        "mapping.get(item) if not __debug__ if item",
        "mapping.get(item) or not __debug__",
        "not (mapping.get(item) or __debug__)",
        "not (mapping.get(item) or __debug__ or not item)",
        "(not __debug__ if mapping.get(item) else item)",
        "(mapping.get(item)\n        and not __debug__\n        and item)",
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!(
            "def collect(mapping, items):\n    result = [item for item in items if {filter}]\n    return result\n"
        );
        assert_matches_cpython(
            &source,
            &format!("folded_comprehension_filter_chain_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn folded_negated_comprehension_filter_preserves_operand_location() {
    for condition in ["not __debug__", "not True", "not False", "False"] {
        let source = format!(
            "def collect(mapping, items):\n    result = [\n        item for item in items\n        if mapping.get(item)\n        and {condition}\n    ]\n    return result\n"
        );
        assert_matches_cpython(&source, "folded_negated_comprehension_filter.py", &LEVELS);
    }
}

#[test]
fn folded_comprehension_and_tail_preserves_borrow_boundary() {
    for (index, source) in [
        "def collect(mapping, items):\n    result = [item for item in items if mapping.get(item) and not __debug__]\n    return result\n",
        "def collect(mapping, items):\n    result = [item for item in items if mapping.get(item) and True]\n    return result\n",
        "def collect(mapping, items):\n    result = [item for item in items if mapping.get(item)]\n    return result\n",
        "def collect(mapping, items):\n    result = [\n        item for item in items\n        if mapping.get(item)\n        and True\n    ]\n    return result\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("folded_comprehension_and_tail_{index}.py"), &LEVELS);
    }
}

#[test]
fn linear_prefix_and_elided_suffix_preserve_nested_if_join() {
    let source = "def update(state):\n    if state.ready():\n        item = state.next()\n        if available(item):\n            apply(item)\n        else:\n            discard(item)\n        assert state.valid()\n    else:\n        assert state.valid()\n    result = state.finish()\n    assert result\n";
    for source in [
        source.to_owned(),
        source.replace(
            "available(item)",
            "all(value == 1 for value in item.values())",
        ),
        source.replace("available(item)", "any(value for value in item.values())"),
        source.replace("available(item)", "tuple(value for value in item.values())"),
        source.replace("        item = state.next()", "        state.prepare()"),
        source.replace(
            "        item = state.next()",
            "        state.prepare()\n        item = state.next()\n        other = item.value",
        ),
        source.replace("        item = state.next()\n", ""),
        source.replacen("        assert state.valid()", "        consume(item)", 1),
    ] {
        assert_matches_cpython(&source, "prefixed_nested_if_suffix.py", &LEVELS);
    }
}

#[test]
fn erased_else_after_breaking_for_preserves_branch_join() {
    for tail in ["finish()", "value = None", "return None"] {
        let source = format!(
            "def f(flag, items):\n    if flag:\n        for item in items:\n            if item:\n                break\n    else:\n        assert flag\n    {tail}\n"
        );
        assert_matches_cpython(&source, "breaking_for_erased_else.py", &LEVELS);
    }
}

#[test]
fn elided_for_after_completed_prefix_preserves_legacy_footer_location() {
    for source in [
        "def f(items, others):\n    assert items\n    assert others\n    if ready():\n        assert items\n    if active():\n        value = prepare()\n        for item in items:\n            assert item\n            assert value\n        for other in others:\n            assert other\n",
        "def f(items, others):\n    assert items\n    assert others\n    if ready():\n        assert items\n    if active():\n        for item in items:\n            assert item\n            assert value\n        for other in others:\n            assert other\n",
        "def f(items, others):\n    assert items\n    assert others\n    if ready():\n        assert items\n    if active():\n        value = prepare()\n        for other in others:\n            assert other\n",
        "def f(items, others):\n    assert items\n    assert others\n    if ready():\n        assert items\n    if active():\n        for other in others:\n            assert other\n",
        "def f(items, others):\n    assert items\n    assert others\n    if active():\n        value = prepare()\n        for item in items:\n            assert item\n            assert value\n        for other in others:\n            assert other\n",
        "def f(items, others):\n    assert items\n    assert others\n    if ready():\n        assert items\n    if active():\n        value = items\n        for item in items:\n            assert item\n            assert value\n        for other in others:\n            assert other\n",
    ] {
        assert_matches_cpython(source, "completed_for_prefix.py", &LEVELS);
    }
    for (target, first, last) in [
        ("item", "items", "range(count)"),
        ("key, value", "items.items()", "others"),
        ("key, value", "items.items()", "range(count)"),
    ] {
        let source = format!(
            "def f(items, others, count):\n    if ready():\n        assert items\n    if active():\n        value = prepare()\n        for {target} in {first}:\n            assert items\n        for other in {last}:\n            assert other\n"
        );
        assert_matches_cpython(&source, "completed_iterator_call_prefix.py", &LEVELS);
    }
}

#[test]
fn elif_footer_keeps_each_predecessors_location_after_removed_assertion() {
    for otherwise in ["assert kind", "value = build()"] {
        let source = format!(
            "def f(flag, kind, build):\n    if flag:\n        if kind == 1:\n            value = build()\n        elif kind == 2:\n            value = build()\n        else:\n            {otherwise}\n        assert flag\n"
        );
        assert_matches_cpython(&source, "elif_footer.py", &LEVELS);
    }
}

#[test]
fn returning_while_uses_condition_location_after_removed_assertion() {
    for condition in ["value >= 0", "count", "value >= 0 or count"] {
        for prefix in [
            "",
            "            if value > 0 and count != 0:\n                count += 1\n                continue\n",
        ] {
            let source = format!(
                "def f(obj, value, count):\n    if value < 0:\n        return obj.first()\n    else:\n        while {condition}:\n{prefix}            assert count\n            return obj.second()\n        assert value\n"
            );
            assert_matches_cpython(&source, "returning_while_footer.py", &LEVELS);
        }
    }
}

#[test]
fn erased_false_branch_preserves_borrowed_local_loads() {
    for condition in ["False", "__debug__", "0", "0x0"] {
        for assertion in ["assert x", "assert x\n            assert y"] {
            let source = format!(
                "def f(a, b, x, y):\n    if a:\n        if {condition}:\n            {assertion}\n        result = consume(x, y)\n    elif b:\n        result = x\n    else:\n        result = None\n    return result\n"
            );
            assert_matches_cpython(&source, "erased_false_borrow.py", &LEVELS);
        }
    }
}

#[test]
fn genuine_pass_preserves_conditional_expression_join() {
    for (index, expression) in [
        "x if b else y",
        "left(x) if b else y",
        "x if b else right(y)",
        "left(x) if b else right(y)",
        "obj.left if b else obj.right",
        "obj.left(x) if b else obj.right(y)",
        "x[0] if b else y[0]",
        "x if b else 1",
        "1 if b else x",
        "1 if b else 2",
        "x if b else 1 + 2",
        "'value' if b else x",
        "x if b else None",
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!(
            "def f(a, b, x, y, left, right, obj):\n    if a:\n        {expression}\n        pass\n"
        );
        assert_matches_cpython(
            &source,
            &format!("conditional_expression_pass_{index}.py"),
            &LEVELS,
        );
    }
    for (expression, suffix) in [
        ("left() if b else right()", ""),
        ("left() if b else right()", "        assert a\n"),
        ("left() if b else right()", "        pass\n    after()\n"),
        ("x if b else 1", "        pass\n    after()\n"),
    ] {
        let source =
            format!("def f(a, b, x, left, right):\n    if a:\n        {expression}\n{suffix}");
        assert_matches_cpython(&source, "conditional_expression_pass_boundary.py", &LEVELS);
    }
}

#[test]
fn erased_suffix_preserves_conditional_expression_join() {
    for condition in ["b", "b is not None"] {
        for (left, right) in [("x", "y"), ("left(x)", "right(y)")] {
            for suffix in ["        assert a\n", "        assert a\n        assert b\n"] {
                let source = format!(
                    "def f(a, b, x, y):\n    if a:\n        {left} if {condition} else {right}\n{suffix}"
                );
                assert_matches_cpython(&source, "conditional_expression_erased_suffix.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_nested_binary_if_preserves_legacy_scope_exit() {
    for (prefix, outer, otherwise) in [
        ("    if a:\n        assert a\n", true, true),
        ("", true, true),
        ("    d = a\n", true, true),
        ("    if a:\n        pass\n", true, true),
        (
            "    if a:\n        assert b\n    else:\n        assert c\n",
            true,
            true,
        ),
        ("    if a:\n        assert a\n", false, true),
        ("    if a:\n        assert a\n", true, false),
        ("    assert a\n", true, true),
    ] {
        let (outer_if, indent) = if outer {
            ("    if b:\n", "        ")
        } else {
            ("", "    ")
        };
        let mut source =
            format!("def f(a, b, c):\n{prefix}{outer_if}{indent}if c:\n{indent}    assert b\n");
        if otherwise {
            writeln!(source, "{indent}else:\n{indent}    assert c").unwrap();
        }
        assert_matches_cpython(&source, "erased_nested_binary_if.py", &LEVELS);
    }
    assert_matches_cpython(
        "def f(a, b):\n    if a.ready:\n        assert b\n    if a is None and b is not None:\n        flag = not a.ready\n        if b.ready:\n            assert a\n        else:\n            assert b\n",
        "erased_nested_binary_if_assignment.py",
        &LEVELS,
    );
}

#[test]
fn attribute_identity_tests_preserve_legacy_scope_exit() {
    let erased_prefix =
        "    if obj.ready:\n        assert left\n        assert right\n        assert obj.valid\n";
    for (condition, prefix) in [
        ("left is None and right is not None", erased_prefix),
        ("obj.left is None and obj.right is not None", erased_prefix),
        ("left is None and obj.right is not None", erased_prefix),
        ("obj.left is None and obj.right is not None", ""),
    ] {
        let source = format!(
            "def f(obj, left, right):\n    if obj.stop:\n        return\n    assert obj.valid\n{prefix}    if {condition}:\n        value = not obj.flag\n        if obj.check:\n            assert value\n        else:\n            assert obj.valid\n"
        );
        assert_matches_cpython(&source, "attribute_identity_scope_exit.py", &LEVELS);
    }
    for assignment in [
        "obj.value = not obj.flag",
        "value = not obj.part.flag",
        "obj.value = not obj.part.flag",
    ] {
        let source = format!(
            "def f(obj, left, right):\n    if obj.stop:\n        return\n{erased_prefix}    if obj.left is None and obj.right is not None:\n        {assignment}\n        if obj.check:\n            assert obj.valid\n        else:\n            assert obj.check\n"
        );
        assert_matches_cpython(&source, "attribute_assignment_scope_exit.py", &LEVELS);
    }
}

#[test]
fn erased_linear_prefix_preserves_populated_nested_join() {
    for prefix in [
        "        assert obj.ready\n        value = prepare(obj)\n",
        "        assert obj.ready\n        pass\n        assert obj.valid\n        value = obj\n",
        "        value = prepare(obj)\n",
    ] {
        let source = format!(
            "def f(obj):\n    if obj is not None:\n{prefix}        if obj.left or obj.right:\n            result = value.left\n        else:\n            result = choose(value)\n            witness = value.right\n    else:\n        assert obj is None\n    witness = obj.after\n"
        );
        assert_matches_cpython(&source, "erased_linear_prefix_join.py", &LEVELS);
    }
}

#[test]
fn raising_arm_preserves_nested_erased_else_join() {
    let source = r#"def configure(enabled, path, options, config):
    if enabled:
        assert path is None
    else:
        if path is None:
            if options.family is None:
                raise ValueError("missing")
            else:
                assert options.name is not None
                assert config is None
                item = lookup(options.family)
                path = item.path
                config = item.config
                if item.layout is not None:
                    options.first = item.layout.first
                    options.second = item.layout.second
                else:
                    assert options.valid
        else:
            assert options.family is None
    result = work(config)
    try:
        finish(result)
    except Error:
        raise
    return result
"#;
    for source in [
        source.to_owned(),
        source.replace("raise ValueError(\"missing\")", "raise error"),
        source.replace("raise ValueError(\"missing\")", "raise error from cause"),
        source.replace("if item.layout is not None:", "if item.ready():"),
    ] {
        assert_matches_cpython(&source, "raising_arm_nested_join.py", &LEVELS);
    }
}

#[test]
fn assigned_call_conditional_prefix_preserves_populated_join() {
    let source = r"def choose(owner, incoming, first):
    if incoming is not None:
        active = owner.enabled()
        assert incoming.valid
        if first:
            value = incoming.prepare(first)
        else:
            value = incoming.prepare(0)
        if value.kind != owner.kind:
            value = value.convert(owner.kind)
        options = configure(callback=lambda: None)
        args = pack(value, options) if owner.recompute or active else None
        if active:
            result = None
        else:
            result = emit(value, args)
    else:
        assert first
    consume(result)
";
    for source in [
        source.to_owned(),
        source.replace("owner.recompute or active", "owner.recompute and active"),
        source.replace("owner.recompute or active", "not owner.ready()"),
        source.replace(
            "pack(value, options) if owner.recompute or active else None",
            "value.left if owner.recompute else value.right",
        ),
    ] {
        assert_matches_cpython(&source, "assigned_call_conditional_prefix.py", &LEVELS);
    }
}

#[test]
fn assigned_conditional_prefix_preserves_populated_nested_join() {
    for successor in ["    after = obj\n", "    after = obj.after\n"] {
        let source = format!(
            "def f(obj, a, b, c, d, work):\n    if obj is not None:\n        if a:\n            first = work()\n        else:\n            first = work()\n        if b:\n            second = work()\n        chosen = c if d else a\n        if c:\n            result = None\n        else:\n            result = work()\n    else:\n        assert obj\n{successor}    return after\n"
        );
        assert_matches_cpython(&source, "assigned_conditional_prefix_join.py", &LEVELS);
    }
}

#[test]
fn conditional_prefix_preserves_erased_nested_join() {
    for prefix in [
        "        if b:\n            value = obj.first\n",
        "        if b:\n            value = obj.first\n        else:\n            value = obj.second\n",
        "        if b:\n            value = obj.first\n            assert b\n        else:\n            value = obj.second\n            assert c\n",
        "        if b:\n            return obj.first\n",
        "        if b:\n            value = obj.first\n        else:\n            return obj.second\n",
        "        if b:\n            value = obj.first\n            return value\n",
        "        if b:\n            return obj.first\n        else:\n            value = obj.second\n",
    ] {
        let source = format!(
            "def f(a, b, c, obj):\n    if a:\n{prefix}        if c:\n            assert c\n        else:\n            assert b\n    else:\n        assert a\n    action()\n"
        );
        assert_matches_cpython(&source, "conditional_prefix_join.py", &LEVELS);
    }
}

#[test]
fn comprehension_and_erased_prefixes_preserve_nested_join() {
    let source = "def collect(kind, items, score, count):\n    if kind == 'selected':\n        assert items\n        values = [item.strip().lower() for item in items]\n        assert values\n        if score < 0:\n            assert values\n        else:\n            assert not values\n        if count > 0:\n            assert values\n        else:\n            assert not values\n        if 'key' in values:\n            assert score\n        else:\n            assert count\n    else:\n        assert not items\n    return {'kind': kind, 'items': items, 'score': score, 'count': count}\n";
    for source in [
        source.to_owned(),
        source.replace(
            "[item.strip().lower() for item in items]",
            "{item.strip().lower() for item in items}",
        ),
        source.replace(
            "[item.strip().lower() for item in items]",
            "{item: item.strip().lower() for item in items}",
        ),
        source.replace(
            "[item.strip().lower() for item in items]",
            "[tuple(item for other in items) for item in items]",
        ),
        source.replace("[item.strip().lower() for item in items]", "items"),
        source.replace(
            "{'kind': kind, 'items': items, 'score': score, 'count': count}",
            "{'items': items}",
        ),
        source.replace(
            "    else:\n        assert not items",
            "    else:\n        consume(items)",
        ),
    ] {
        assert_matches_cpython(&source, "comprehension_erased_prefixes.py", &LEVELS);
    }
}

#[test]
fn erased_prefix_preserves_constructed_raise_return_copies() {
    const SOURCE: &str = "def f(obj):\n    if obj.count is not None:\n        assert obj.ready\n    if check(obj.child):\n        if obj.count is not None:\n            raise ValueError(\"count\")\n        values = obj.child.values\n        if any(value % obj.scale for value in values):\n            raise ValueError(\"scale\")\n        if any(value < obj.lower for value in values):\n            raise ValueError(\"lower\")\n        if obj.upper is not None and any(value > obj.upper for value in values):\n            raise ValueError(\"upper\")\n";
    let prefix = "    if obj.count is not None:\n        assert obj.ready\n";
    let mut method = "class C:\n".to_owned();
    for line in SOURCE.lines() {
        writeln!(method, "    {line}").unwrap();
    }
    for (index, source) in [
        SOURCE.to_owned(),
        SOURCE
            .replace("any(value % obj.scale for value in values)", "check(values)")
            .replace("any(value < obj.lower for value in values)", "check(values)")
            .replace("any(value > obj.upper for value in values)", "check(values)"),
        SOURCE.replace("check(obj.child)", "check(obj.child, obj.count, ready=True)"),
        SOURCE.replace("value > obj.upper for value in values", "(value if value else obj.upper) > obj.upper for value in values if value and obj.ready"),
        SOURCE.replace(prefix, ""),
        SOURCE.replace("        assert obj.ready\n", "        pass\n"),
        SOURCE.replace("        assert obj.ready\n", "        obj.prepare()\n"),
        format!("{SOURCE}    finish()\n"),
        "def f(a, b, c):\n    if a:\n        assert a\n    if b:\n        if c:\n            raise ValueError(\"bad\")\n".to_owned(),
        "def f(obj):\n    if obj.ready:\n        assert obj.ready\n    if obj.open:\n        if obj.bad:\n            raise obj.error\n".to_owned(),
        method,
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("constructed_raise_scope_exit_{index}.py"), &LEVELS);
    }
}

#[test]
fn erased_condition_before_nested_raise_preserves_scope_exit() {
    for prefix in [
        "    if a:\n        assert a\n",
        "",
        "    if a:\n        pass\n",
        "    assert a\n",
        "    if a:\n        assert b\n    else:\n        assert c\n",
    ] {
        let source = format!(
            "def f(a, b, c):\n{prefix}    if b:\n        if c:\n            raise ValueError\n"
        );
        assert_matches_cpython(&source, "nested_raise.py", &LEVELS);
    }
    for tail in ["c", "b and c"] {
        let source = format!(
            "def f(a, b, c):\n    if a:\n        assert a\n    if b:\n        if {tail}:\n            raise ValueError\n"
        );
        assert_matches_cpython(&source, "nested_raise_condition.py", &LEVELS);
    }
    for guards in 0..=3 {
        for assignment in [false, true] {
            let mut source = "def f(a, obj, enter, check, finish):\n    if a is not None:\n        assert a\n    if enter(obj):\n".to_owned();
            for (index, condition) in ["a", "check(obj)", "a is not None"]
                .into_iter()
                .take(guards)
                .enumerate()
            {
                writeln!(
                    source,
                    "        if {condition}:\n            raise ValueError"
                )
                .unwrap();
                if assignment && index == 0 {
                    source.push_str("        value = obj.value\n");
                }
            }
            if assignment && guards == 0 {
                source.push_str("        value = obj.value\n");
            }
            source.push_str(
                "        if a is not None and finish(obj):\n            raise ValueError\n",
            );
            assert_matches_cpython(&source, "nested_raise_guards.py", &LEVELS);
        }
    }
    for source in [
        "def f(a, b, c):\n    if a:\n        assert a\n    if b:\n        if c:\n            pass\n",
        "def f(a, b, c):\n    if a:\n        assert a\n    if b:\n        if c:\n            raise ValueError\n    return None\n",
        "def f(a, obj, finish):\n    if a is not None:\n        assert a\n    if a is not None and finish(obj):\n        raise ValueError\n",
    ] {
        assert_matches_cpython(source, "nested_raise_control.py", &LEVELS);
    }
}

#[test]
fn populated_nested_elif_preserves_outer_join() {
    let source = r#"def select(outer, first, second, flag, value, action):
    if outer:
        count = action()
        if first is not None:
            assert value is None
            values = [count]
            value = action(values)
        else:
            if second is not None:
                assert value.ready
                assert value.valid
                if value.stride(-1) != 1:
                    raise ValueError("invalid stride")
                value = value.T
            elif flag:
                value = action(count)
            else:
                value = action(count)
                value = value.T
    else:
        assert first is None
    action(value)
"#;
    for source in [
        source.to_owned(),
        source.replace(
            "                if value.stride(-1) != 1:\n                    raise ValueError(\"invalid stride\")\n",
            "",
        ),
        source.replace("        assert first is None", "        action(first)"),
    ] {
        assert_matches_cpython(&source, "populated_nested_elif_join.py", &LEVELS);
    }
}

#[test]
fn generator_call_stores_preserve_populated_nested_join() {
    let source = r"def choose(result, items):
    if result is None:
        flag = any(check(item) for item in items)
        if flag:
            assert all(check(item) for item in items)
            selected = next(item for item in items if check(item))
            result = build(selected)
        else:
            result = fallback(items)
    else:
        assert valid(result)
    consume(result)
    return result
";
    let ordinary_prefix = source.replace("any(check(item) for item in items)", "check(items)");
    let without_assertion = |source: &str| {
        source.replace(
            "            assert all(check(item) for item in items)\n",
            "",
        )
    };
    let linear_selection = |source: &str| {
        source.replace(
            "next(item for item in items if check(item))",
            "select(items)",
        )
    };
    for source in [
        source.to_owned(),
        linear_selection(&without_assertion(source)),
        without_assertion(&ordinary_prefix),
        linear_selection(&ordinary_prefix),
        linear_selection(&without_assertion(&ordinary_prefix)),
        source.replace("        assert valid(result)", "        consume(result)"),
    ] {
        assert_matches_cpython(&source, "generator_call_store_join.py", &LEVELS);
    }
}

#[test]
fn completed_comprehension_preserves_nested_branch_join() {
    let source = "def collect(options, values):\n    result = options.values\n    if result is None:\n        if values is not None:\n            result = [item.value for item in values]\n        else:\n            result = make_values()\n    else:\n        assert result\n    output = []\n    for item in result:\n        output.append(item)\n    return output\n";
    for source in [
        source.to_owned(),
        source.replace(
            "[item.value for item in values]",
            "[key for key, item in values.items() if item.enabled]",
        ),
        source.replace(
            "[item.value for item in values]",
            "{item.value for item in values}",
        ),
        source.replace(
            "[item.value for item in values]",
            "{item.key: item.value for item in values}",
        ),
        source.replace(
            "[item.value for item in values]",
            "[(item, other) for item in values if item for other in values if other]",
        ),
        source.replace(
            "[item.value for item in values]",
            "[tuple(item for other in values) for item in values]",
        ),
        source.replace(
            "            result = [item.value for item in values]",
            "            prepared = [item.value for item in values]\n            result = finish(prepared)",
        ),
        source.replace("        assert result", "        consume(result)"),
        source[..source.find("    output = []").unwrap()].to_owned(),
    ] {
        assert_matches_cpython(&source, "completed_comprehension_join.py", &LEVELS);
    }
}

#[test]
fn completed_loop_preserves_nested_branch_join() {
    let source = r#"def render(outputs, allowed):
    if outputs is None:
        prefix = "empty"
    elif isinstance(outputs, tuple):
        prefix = f"{outputs!r}"
    elif isinstance(outputs, dict):
        values = [value for value in outputs.values() if ready(value)]
        if values and all(value in allowed for value in values):
            prefix = f"{outputs!r}"
        else:
            token = make()
            prefix = f"{show(token)} = call"
            for name, value in outputs.items():
                if not ready(value):
                    continue
                register(value, f"{show(token)}[{show(name)}]")
    else:
        assert False
    consume(prefix, outputs, allowed)
    return prefix
"#;
    for source in [
        source.to_owned(),
        source.replace("values and all", "values or all"),
        source.replace(
            "if values and all(value in allowed for value in values):",
            "if not (values and all(value in allowed for value in values)):",
        ),
        source.replace(
            "                if not ready(value):\n                    continue\n",
            "",
        ),
    ] {
        assert_matches_cpython(&source, "completed_loop_join.py", &LEVELS);
    }
}

#[test]
fn conditional_prefixes_with_erased_else_preserve_nested_join() {
    for prefix in [
        "        if b:\n            value = obj.first\n        if c:\n            value = obj.second\n",
        "        if b:\n            value = obj.first\n        if c:\n            value = obj.second\n        else:\n            assert c\n",
        "        if b:\n            value = obj.first\n        else:\n            assert b\n        if c:\n            value = obj.second\n",
        "        if b:\n            value = obj.first\n        else:\n            assert b\n",
        "        if b:\n            value = obj.first\n        if c:\n            value = obj.second\n        else:\n            value = obj.third\n",
        "        if b:\n            value = obj.first\n        else:\n            assert b\n        if c:\n            value = obj.second\n        else:\n            assert c\n",
    ] {
        let source = format!(
            "def f(a, b, c, obj):\n    if a:\n{prefix}        if c:\n            assert c\n        else:\n            assert b\n    else:\n        assert a\n    action()\n"
        );
        assert_matches_cpython(&source, "conditional_prefix_elses.py", &LEVELS);
    }
}

#[test]
fn conditional_infinite_loop_preserves_shared_scope_return() {
    let source = "def f(state, work):\n    if state.check:\n        assert state.valid\n    work()\n    if not state.check:\n        while True:\n            try:\n                value = work()\n            except ValueError as error:\n                report(error)\n            if not state.repeat:\n                return value.result\n";
    for source in [
        source.to_owned(),
        source.replace("    if state.check:\n        assert state.valid\n", ""),
        source.replace("while True:", "while state.running:"),
        source
            .replace("except ValueError as error:", "except ValueError:")
            .replace("report(error)", "report()"),
        format!("{source}    work()\n"),
        source.replace(
            "        while True:\n",
            "        while True:\n            for item in work():\n                break\n",
        ),
    ] {
        assert_matches_cpython(&source, "conditional_infinite_loop_footer.py", &LEVELS);
    }
}

#[test]
fn module_terminal_calls_preserve_shared_return_locations() {
    let source = "if __name__ == \"__main__\":\n    if not wants_help():\n        assert ready()\n    launch(\n        value=config,\n    )\n";
    for source in [
        source.to_owned(),
        source.replace("launch(", "worker.launch("),
        source.replace("assert ready()", "pass"),
        format!("{source}consume(config)\n"),
        format!(
            "\"description\"\nfrom __future__ import annotations\nimport library\ndef prepare(value: int) -> None:\n    consume(value)\n\n{source}"
        ),
    ] {
        assert_matches_cpython(&source, "module_call_footer.py", &LEVELS);
    }
}

#[test]
fn coroutine_call_footer_preserves_original_location() {
    for prefix in [
        "",
        "    await prepare()\n",
        "    items = [item.value for item in obj]\n",
    ] {
        for prior in ["", "    if value in {1, 2}:\n        assert obj\n"] {
            let source = format!(
                "async def f(value, obj):\n{prefix}{prior}    if value == 3:\n        assert obj\n        obj.work()\n"
            );
            assert_matches_cpython(&source, "coroutine_call_footer.py", &LEVELS);
        }
    }
}

#[test]
fn async_manager_setups_preserve_repeated_loop_backedges() {
    let prefix = "    async with manager:\n        value = await worker()\n";
    let source = format!(
        "async def f(manager, worker, items):\n{prefix}    for item in items:\n        assert item\n        if item.kind == 1:\n            assert value\n        if item.kind == 0:\n            assert value\n"
    );
    for source in [
        source.clone(),
        source.replace(prefix, "    async with manager:\n        async with manager:\n            value = await worker()\n"),
        source.replace("    for item", "        for item")
            .replace("        assert item", "            assert item")
            .replace("        if item", "            if item")
            .replace("            assert value", "                assert value"),
        format!("{source}        worker()\n"),
        format!("{source}    worker()\n"),
        format!("async def f(manager, worker, first, second):\n{prefix}    if first:\n        assert second\n    if second:\n        worker()\n"),
    ] {
        assert_matches_cpython(&source, "async_manager_loop_backedge.py", &LEVELS);
    }
}

#[test]
fn coroutine_erased_branches_preserve_shared_return_locations() {
    let definition = "    def choose(value):\n        if value:\n            return first\n        return False\n";
    let source = format!(
        "async def f(work, first, second):\n{definition}    await work(choose)\n    value = await work()\n    assert value\n    if first is not None:\n        assert value\n    if second:\n        assert value\n    else:\n        assert value\n"
    );
    for source in [
        source.clone(),
        source
            .replace(definition, "")
            .replace("work(choose)", "work(first)"),
        source.replace("await ", ""),
        source.replace("    if first is not None:\n        assert value\n", ""),
        source.replace(
            "    else:\n        assert value\n",
            "    else:\n        pass\n",
        ),
        source.replace("    if second:", "    if not second:"),
    ] {
        assert_matches_cpython(&source, "coroutine_erased_branches_footer.py", &LEVELS);
    }
}

#[test]
fn coroutine_erased_else_preserves_shared_return_locations() {
    let source = "async def f(manager, worker, first, second, items):\n    with manager:\n        await worker()\n    values = {item: item for item in items}\n    item = values[0]\n    if first:\n        assert item\n    if second:\n        assert item\n        value = values[1]\n        assert value\n    else:\n        assert item\n";
    for source in [
        source.to_owned(),
        source.replace(
            "    with manager:\n        await worker()\n    values = {item: item for item in items}\n",
            "    values = items\n",
        ),
        source.replace("value = values[1]", "value = worker()"),
        source.replace("    else:\n        assert item", "    else:\n        pass"),
        source.replace("    else:\n        assert item", "    else:\n        worker()"),
        format!("{source}    worker()\n"),
    ] {
        assert_matches_cpython(&source, "coroutine_erased_else_footer.py", &LEVELS);
    }
}

#[test]
fn erased_assertions_inside_terminal_call_suite_preserve_footer() {
    for prefix in [
        "    if a:\n        assert a\n",
        "",
        "    if a:\n        pass\n",
        "    assert a\n",
        "    if a:\n        assert a\n    else:\n        assert b\n",
    ] {
        for body in [
            "        assert b\n        finish(obj)\n",
            "        value = obj.left\n        assert value\n        value = obj.right\n        finish(value)\n",
            "        pass\n        assert b\n        finish(obj)\n",
            "        17\n        assert b\n        finish(obj)\n",
            "        assert a\n        value = obj.left\n        assert value\n        obj.finish(value)\n        assert b\n",
        ] {
            let source = format!("def f(a, b, obj, finish):\n{prefix}    if b:\n{body}");
            assert_matches_cpython(&source, "internal_erased_assert_call.py", &LEVELS);
        }
    }
}

#[test]
fn elided_condition_before_terminal_for_preserves_legacy_footer_location() {
    for first in ["a", "check(a)"] {
        for second in ["b", "check(b)"] {
            for loop_prefix in ["", "        assert a\n"] {
                let source = format!(
                    "def f(a, b, items):\n    assert a\n    if {first}:\n        assert a\n    if {second}:\n{loop_prefix}        for item in items:\n            assert item\n"
                );
                assert_matches_cpython(&source, "elided_condition_loop_footer.py", &LEVELS);
            }
        }
    }
}

#[test]
fn attribute_iterator_preserves_original_footer_location() {
    for prefix in ["    if obj.ready:\n        assert obj.ready\n", ""] {
        let source = format!(
            "def f(obj):\n{prefix}    if obj.active and obj.target is not None:\n        for item in obj.items:\n            item.action()\n"
        );
        assert_matches_cpython(&source, "attribute_iterator_footer.py", &LEVELS);
    }
}

#[test]
fn method_iterator_preserves_original_footer_location() {
    for (target, iterator, following) in [
        ("item", "obj.items()", ""),
        ("key, item", "obj.values.items()", ""),
        ("key, item", "obj.values.items(argument)", ""),
        ("key, item", "items(argument)", ""),
        ("key, item", "values", ""),
        ("key, item", "obj.values", ""),
        ("key, item", "obj.values.items()", "    finish()\n"),
    ] {
        let source = format!(
            "def f(obj, items, values, argument):\n    if obj.optional is not None:\n        assert obj\n    if obj.values:\n        for {target} in {iterator}:\n            assert item\n{following}"
        );
        assert_matches_cpython(&source, "method_iterator_footer.py", &LEVELS);
    }
}

#[test]
fn linear_for_method_arguments_preserve_original_footer_location() {
    for arguments in [
        "argument",
        "argument, other",
        "argument.value",
        "produce(argument)",
        "argument=argument",
        "*argument",
        "__debug__",
    ] {
        let source = format!(
            "def f(obj, argument, other):\n    if obj.ready:\n        assert obj.ready\n    if obj.active and obj.target is not None:\n        for item in obj.items:\n            item.action({arguments})\n"
        );
        assert_matches_cpython(&source, "linear_for_method_arguments.py", &LEVELS);
    }
}

#[test]
fn linear_for_unpacking_preserves_original_footer_location() {
    for (target, iterator, body) in [
        ("item", "make_items(items, argument)", "assert argument"),
        ("item, other", "items", "assert argument"),
        (
            "item, other",
            "make_items(items, argument=argument)",
            "assert argument",
        ),
        ("()", "make_items(items, argument)", "assert argument"),
        (
            "item, other",
            "make_items(items, argument.extra)",
            "item.action(argument)",
        ),
    ] {
        let source = format!(
            "def f(obj, items, argument):\n    if obj.ready:\n        assert obj.ready\n    if obj.active and obj.target is not None:\n        for {target} in {iterator}:\n            {body}\n"
        );
        assert_matches_cpython(&source, "linear_for_unpacking.py", &LEVELS);
    }
}

#[test]
fn elided_assertion_footer_inherits_live_branch_location() {
    for exit in ["raise ValueError", "return", "return b"] {
        for suffix in [
            "",
            "        assert a\n",
            "        assert a\n        assert b\n",
        ] {
            let source = format!(
                "def f(a, b):\n    if a:\n        if b:\n            value = action()\n        else:\n            {exit}\n{suffix}    else:\n        raise RuntimeError\n"
            );
            assert_matches_cpython(&source, "elided_assertion_footer.py", &LEVELS);
        }
    }
}

#[test]
fn elided_call_suffix_preserves_legacy_footer_location() {
    for (first, second) in [("a", "b"), ("a is not None", "b is not None")] {
        for call in ["action(obj)", "obj.update(b)"] {
            for suffix in [
                "        assert obj\n",
                "        assert obj\n        assert b\n",
            ] {
                let source = format!(
                    "def f(a, b, obj):\n    if {first}:\n        assert a\n    if {second}:\n        value = factory(b)\n        {call}\n{suffix}"
                );
                assert_matches_cpython(&source, "elided_call_suffix.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_async_for_assertions_preserve_continue_tail() {
    for prefix in ["", "        total += 1\n"] {
        for work in ["", "            skipped += 1\n"] {
            for suffix in ["", "        assert item\n        assert total\n"] {
                let source = format!(
                    "async def f(items):\n    total = 0\n    skipped = 0\n    async for item in items:\n{prefix}        if item is None:\n{work}            continue\n{suffix}"
                );
                assert_matches_cpython(&source, "async_for_continue_tail.py", &LEVELS);
            }
        }
    }
}

#[test]
fn elided_assertions_preserve_terminal_call_join_boundaries() {
    for call in [
        "sink(x if flag else y)",
        "sink.call(x, y, x if flag else y)",
    ] {
        for suffix in ["        assert x\n", "        assert x\n        assert y\n"] {
            for tail in [
                "",
                "    else:\n        sink(x)\n",
                "    elif flag:\n        sink(x if tag else y)\n        assert y\n    else:\n        sink(y)\n",
            ] {
                let source = format!(
                    "def f(tag, sink, x, y, flag):\n    if tag:\n        {call}\n{suffix}{tail}"
                );
                assert_matches_cpython(&source, "elided_assertion_call_join.py", &LEVELS);
            }
        }
    }
}

#[test]
fn final_elif_preserves_nested_assignment_join() {
    for prefix in [
        "    if a == 1:\n        value = obj.first\n",
        "    if a == 0:\n        return obj.first\n    elif a == 1:\n        value = obj.first\n",
    ] {
        for condition in ["b", "b and c", "b or c"] {
            let source = format!(
                "def f(a, b, c, obj):\n{prefix}    elif a == 2:\n        if {condition}:\n            value = obj.left\n        else:\n            value = obj.right\n    else:\n        assert a\n    action()\n"
            );
            assert_matches_cpython(&source, "final_elif_nested_join.py", &LEVELS);
        }
    }
}

#[test]
fn ordinary_await_preserves_repeated_condition_backedge_locations() {
    for (before, inside, after) in [
        ("    await a\n", "", ""),
        ("", "        await a\n", ""),
        ("", "", "    await a\n"),
        ("    await a\n    await b\n", "", ""),
    ] {
        let source = format!(
            "async def f(items, a, b):\n{before}    for item in items:\n{inside}        if a:\n            assert item\n        if b:\n            assert item\n{after}"
        );
        assert_matches_cpython(&source, "ordinary_await_backedge.py", &LEVELS);
    }
}

#[test]
fn sync_manager_preserves_elided_branch_pop_block() {
    for (manager, indent) in [
        ("    with cm:\n", "        "),
        ("    with cm, other:\n", "        "),
        ("    with cm:\n        with other:\n", "            "),
    ] {
        for prefix in [
            "",
            "try:\n    work()\nexcept ValueError:\n    recover()\nwork()\n",
        ] {
            for (true_arm, false_arm) in [
                ("assert flag", "raise ValueError"),
                ("raise ValueError", "assert flag"),
            ] {
                let body = format!("{prefix}if flag:\n    {true_arm}\nelse:\n    {false_arm}\n");
                let mut source = format!("def f(cm, other, flag):\n{manager}");
                for line in body.lines() {
                    source.push_str(indent);
                    source.push_str(line);
                    source.push('\n');
                }
                assert_matches_cpython(&source, "manager_elided_receiver.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_match_case_suffix_preserves_join_before_successor() {
    for scope in ["def", "async def"] {
        for assertion in ["            assert c\n", "\n"] {
            for successor in [
                "    action()\n",
                "    with cm:\n        action()\n",
                "    try:\n        action()\n    except ValueError:\n        recover()\n",
                "    if b:\n        action()\n",
            ] {
                let source = format!(
                    "{scope} f(a, b, c, cm):\n    match a:\n        case 1:\n            if c:\n                action()\n{assertion}{successor}"
                );
                assert_matches_cpython(&source, "match_case_erased_suffix.py", &LEVELS);
            }
        }
    }
}

#[test]
fn coroutine_sequential_conditions_preserve_erased_joins() {
    for next_test in ["second", "second and third", "second or third"] {
        let source = format!(
            "async def f(items, first, second, third):\n    await prepare()\n    for item in items:\n        if first:\n            assert item\n        else:\n            assert items\n        if {next_test}:\n            assert item\n        else:\n            assert items\n"
        );
        assert_matches_cpython(&source, "coroutine_sequential_joins.py", &LEVELS);
    }
}

#[test]
fn coroutine_loop_store_preserves_elided_join_nop() {
    for target in ["item.state", "item.parent.state", "item[0]"] {
        let source = format!(
            "async def f(items):\n    await prepare()\n    for item in items:\n        if item.kind == 1:\n            assert item\n        else:\n            assert items\n        {target} = None\n        assert item\n"
        );
        assert_matches_cpython(&source, "coroutine_loop_store_join.py", &LEVELS);
    }
}

#[test]
fn coroutine_linear_prefix_preserves_nested_elided_join_nop() {
    for manager in ["with", "async with"] {
        for prefix in [
            "value = current.state.value",
            "current.value = result",
            "value = await read(current)",
        ] {
            let source = format!(
                "async def inspect(manager, enabled, kind, flag):\n    {manager} manager as current:\n        result = await current.read()\n        if enabled and kind == 'ready':\n            assert result\n            {prefix}\n            if flag:\n                assert result\n            else:\n                assert not result\n            assert result\n        else:\n            assert not result\n"
            );
            assert_matches_cpython(&source, "coroutine_prefix_nested_join.py", &LEVELS);
        }
    }
}

#[test]
fn coroutine_manager_preserves_nested_elided_join_nop() {
    for manager in ["with", "async with"] {
        for prefix in ["", "        work()\n", "        await work()\n"] {
            for continuation in ["", "        work()\n"] {
                let source = format!(
                    "async def f(cm, a, b):\n    {manager} cm:\n{prefix}        if a:\n            if b:\n                assert b\n            else:\n                assert a\n        else:\n            assert a\n{continuation}"
                );
                assert_matches_cpython(&source, "coroutine_manager_nested_join.py", &LEVELS);
            }
        }
    }
}

#[test]
fn elided_non_final_disjunction_keeps_backedge_copy_order() {
    for (case, source) in [
        "def f(items, flag, a, b):\n    for item in items:\n        if flag:\n            if a or b:\n                assert item\n        else:\n            work(item)\n",
        "def f(items, flag, a, b, c):\n    for item in items:\n        if flag:\n            if a or b or c:\n                assert item\n        else:\n            pass\n",
        "def f(items, flag, a, b):\n    for outer in items:\n        for item in items:\n            if flag:\n                if a == 1 or check(b):\n                    assert item\n            else:\n                work(item)\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("elided_non_final_disjunction_{case}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_assertions_preserve_raising_try_guard_exit_locations() {
    for scope in ["def", "async def"] {
        for condition in ["flag", "not flag", "flag or other"] {
            for suffix in ["", "        assert flag\n        assert other, explain()\n"] {
                let source = format!(
                    "{scope} f(flag, other):\n    try:\n        if {condition}:\n            raise ValueError(flag) from other\n{suffix}    except Exception:\n        recover()\n    finish()\n"
                );
                assert_matches_cpython(&source, "erased_raising_guard_exit.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_assertions_preserve_raising_try_else_guard_exit_locations() {
    for scope in ["def", "async def"] {
        for condition in ["flag is not None", "not flag", "flag or other"] {
            for suffix in ["", "        assert flag\n        assert other, explain()\n"] {
                let source = format!(
                    "{scope} f(flag, other):\n    try:\n        work()\n    except Exception:\n        recover()\n    else:\n        if {condition}:\n            raise ValueError(flag) from other\n{suffix}    finish()\n"
                );
                assert_matches_cpython(&source, "erased_raising_else_guard_exit.py", &LEVELS);
            }
        }
    }
}

#[test]
fn nested_boolean_assignment_joins_preserve_nops_before_elided_else() {
    for condition in ["b or c", "b and c", "(b or c) and d", "not (b and c)"] {
        for continuation in ["work(value)", "return value"] {
            let source = format!(
                "def f(a, b, c, d):\n    if a:\n        if {condition}:\n            value = 1\n        else:\n            value = 2\n    else:\n        assert a\n    {continuation}\n"
            );
            assert_matches_cpython(&source, "nested_boolean_assignment_join.py", &LEVELS);
        }
    }
}

#[test]
fn nested_assignment_joins_preserve_nops_before_elided_else() {
    for prefix in ["", "        value = 0\n", "        work()\n"] {
        for true_arm in ["assert b", "value = 1"] {
            for continuation in ["work()", "return value"] {
                let source = format!(
                    "def f(a, b):\n    if a:\n{prefix}        if b:\n            {true_arm}\n        else:\n            value = 2\n    else:\n        assert a\n    {continuation}\n"
                );
                assert_matches_cpython(&source, "nested_assignment_join.py", &LEVELS);
            }
        }
    }
}

#[test]
fn elided_else_preserves_the_original_legacy_scope_footer() {
    for condition in ["b", "b and c", "not (b and c)"] {
        for body in ["assert b", "obj.action()"] {
            let source = format!(
                "def f(a, b, c, obj):\n    if a:\n        assert a\n    if {condition}:\n        {body}\n    else:\n        assert c\n"
            );
            assert_matches_cpython(&source, "elided_else_scope_footer.py", &LEVELS);
        }
    }
}

#[test]
fn terminal_finally_and_loop_preserves_returns_before_removed_assertions() {
    for (index, source) in [
        "def f(a, b):\n    try:\n        action()\n    finally:\n        while a and b:\n            action()\n    assert a\n",
        "def f(a, b, c):\n    try:\n        value = action()\n    finally:\n        while a and b and c:\n            b = step()\n    assert a\n    assert b\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("finally_and_assert_return_{index}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn terminal_finally_loop_after_linear_cleanup_preserves_returns() {
    let source = "def f(state):\n    try:\n        value = work()\n        assert value\n    finally:\n        state.close()\n        deadline = clock() + 2\n        while state.active and clock() < deadline:\n            pause()\n    assert not state.active\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("        assert value\n", ""),
        source.replace(
            "        state.close()\n        deadline = clock() + 2\n",
            "",
        ),
        source.replace("    assert not state.active\n", "    consume(state)\n"),
        source.replace(
            "            pause()\n",
            "            pause()\n        finish()\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("finally_loop_cleanup_return_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn terminating_if_before_erased_assertions_preserves_the_legacy_footer() {
    for body in [
        "return None",
        "return obj",
        "raise ValueError",
        "value = obj.attr\n        return value",
    ] {
        let source = format!(
            "def f(a, b, obj):\n    if a:\n        assert a\n    if b:\n        {body}\n    assert b\n"
        );
        assert_matches_cpython(&source, "terminating_if_erased_footer.py", &LEVELS);
    }
}

#[test]
fn erased_assertion_gaps_preserve_conditional_join_nops() {
    for gap in [
        "",
        "        assert flag\n",
        "        assert flag, describe()\n        assert (hidden := flag)\n",
    ] {
        for successor in ["work()", "value = flag", "del flag"] {
            let source = format!(
                "def f(flag):\n    try:\n        if flag:\n            assert flag\n        else:\n            assert not flag\n{gap}        {successor}\n    except ValueError:\n        recover()\n"
            );
            assert_matches_cpython(&source, "erased_assertion_join_gap.py", &LEVELS);
        }
    }
}

#[test]
fn comprehension_setups_preserve_repeated_legacy_jump_locations() {
    for (case, source) in [
        "def f(items, a, b):\n    for item in items:\n        [x for x in item]\n        if a:\n            assert a\n        if b:\n            assert b\n",
        "def f(items, a, b):\n    value = {x for x in items}\n    for item in items:\n        if a:\n            assert a\n        if b:\n            assert b\n",
        "def f(items, a, b):\n    if a:\n        assert a\n    if b:\n        assert b\n    else:\n        [x for x in items]\n    a+a\n",
        "def f(items, a, b):\n    for item in items:\n        [[y for y in x] for x in items]\n        if a:\n            assert a\n        if b:\n            assert b\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("comprehension_repeated_jump_{case}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn manager_setups_preserve_repeated_legacy_empty_branch_locations() {
    for (case, source) in [
        "def f(a, b, c, obj):\n    with obj:\n        value = obj.action()\n    if a:\n        assert a\n    if b:\n        assert b\n    else:\n        assert c\n    obj.action()\n",
        "def f(a, b, obj):\n    with obj:\n        if a:\n            assert a\n        if b:\n            assert b\n        else:\n            assert a\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("manager_repeated_empty_branch_{case}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn direct_finally_async_for_keeps_exit_before_erased_assertions() {
    for (index, source) in [
        "async def f(items, obj):\n    try:\n        async for item in items:\n            pass\n        assert items\n    finally:\n        obj.finish()\n",
        "async def f(items, obj):\n    try:\n        async for item in items:\n            obj.accept(item)\n        assert items\n    finally:\n        await obj.finish()\n",
        "async def f(items, obj):\n    try:\n        token = prepare()\n        obj.begin()\n        items = adapt(items)\n        async for item in items:\n            pass\n        assert items\n    finally:\n        obj.finish()\n",
        "async def f(items, obj):\n    try:\n        async for item in items:\n            pass\n    finally:\n        obj.finish()\n",
        "async def f(items, obj):\n    try:\n        async for item in adapt(\n            items\n        ):\n            pass\n        assert items\n    finally:\n        obj.finish()\n",
        "async def f(items, obj):\n    try:\n        async for item in items:\n            pass\n        assert items\n        async for again in items:\n            obj.accept(again)\n        assert items\n    finally:\n        obj.finish()\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("direct_finally_async_for_exit_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_else_preserves_final_elif_try_exit_locations() {
    for condition in [
        "second(value)",
        "not second(value)",
        "all(valid(item) for item in items)",
    ] {
        for value in ["transform(items)", "[transform(item) for item in items]"] {
            let source = format!(
                "def f(items, value):\n    try:\n        if first(value):\n            result = transform(items)\n        elif {condition}:\n            result = {value}\n        else:\n            assert valid(value)\n    except ValueError as error:\n        raise TypeError(value) from error\n    consume(result)\n    return result\n"
            );
            assert_matches_cpython(&source, "erased_elif_try_exit.py", &LEVELS);
        }
    }
}

#[test]
fn erased_try_else_keeps_condition_location_on_retained_exit() {
    for (case, source) in [
        "def f(obj, flag):\n    try:\n        if flag:\n            obj.action()\n        else:\n            assert flag\n    except ValueError:\n        return False\n    if flag:\n        assert obj\n    return True\n",
        "def f(a,b,obj,cm,items):\n    try:\n        if not a:\n            obj.action()\n        else:\n            assert a\n    except ValueError:\n        return False\n    if a:\n        assert obj\n    return True\n",
        "def f(a,b,obj,cm,items):\n    try:\n        if a or b:\n            obj.action()\n        else:\n            assert a\n    except ValueError:\n        return False\n    if a:\n        assert obj\n    return True\n",
        "def f(a,b,obj,cm,items):\n    try:\n        if a == b:\n            value = obj.value\n        else:\n            assert a\n    except ValueError:\n        return False\n    if a:\n        assert obj\n    return True\n",
        "def f(a,b,obj,cm,items):\n    try:\n        action()\n        if a:\n            obj.action()\n        else:\n            assert a\n    except ValueError:\n        return False\n    if a:\n        assert obj\n    return True\n",
        "def f(a,b,obj,cm,items):\n    try:\n        if a:\n            obj.action()\n        else:\n            assert a\n    except ValueError as exc:\n        recover(exc)\n    if a:\n        assert obj\n    return True\n",
        "def f(a,b,obj,cm,items):\n    try:\n        if a:\n            obj.action()\n        else:\n            assert a\n    except ValueError:\n        return False\n    action()\n    return obj\n",
        "def f(a,b,obj,cm,items):\n    try:\n        if a:\n            obj.action()\n        else:\n            assert a, describe()\n    except ValueError:\n        return False\n    if a:\n        assert obj\n    return True\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("erased_try_else_exit_{case}.py"), &LEVELS);
    }
}

#[test]
fn legacy_coroutine_conditional_work_keeps_returns_before_erased_assertions() {
    for (case, source) in [
        "async def f(a, b, items):\n    if a:\n        if b:\n            value = [x for x in items]\n        assert a\n",
        "async def f(a, b, items):\n    if a:\n        return\n    else:\n        if b:\n            value = {x for x in items}\n        assert a\n",
        "async def f(a, b, items):\n    await action()\n    if a:\n        if b:\n            value = {x: a for x in items}\n        assert a\n",
        "async def f(a, b, obj):\n    if a:\n        if b:\n            value = [x for x in (a, b)]\n        assert a\n",
        "async def f(a, b, obj):\n    if a:\n        if b:\n            if obj:\n                action()\n        assert a\n",
        "async def f(a, b, obj, items):\n    if a:\n        if b:\n            value = [x for x in items]\n            if obj:\n                action(value)\n            value = [x for x in items]\n        assert a\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("legacy_coroutine_conditional_work_{case}.py"),
            &LEVELS,
            &[PythonVersion::Py313],
        );
    }
}
#[test]
fn terminating_legacy_branches_preserve_the_original_scope_footer() {
    for (index, tail) in [
        "raise ValueError",
        "return obj",
        "return 1",
        "return",
        "value = obj.attr",
    ]
    .into_iter()
    .enumerate()
    {
        let source =
            format!("def f(a, b, obj):\n    if a:\n        assert a\n    if b:\n        {tail}\n");
        assert_matches_cpython(
            &source,
            &format!("original_legacy_scope_footer_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn elided_assertions_preserve_legacy_nested_call_footers() {
    for condition in ["c", "c and a"] {
        for call in ["obj.action()", "obj.action(*items)"] {
            let source = format!(
                "def f(a, b, c, obj, items):\n    if a:\n        assert a\n    if b:\n        if {condition}:\n            {call}\n        assert b\n"
            );
            assert_matches_cpython(&source, "legacy_nested_call_footer.py", &LEVELS);
        }
    }
    assert_matches_cpython(
        "def f(a, b, obj):\n    if a:\n        assert a\n    if b:\n        value = obj.attr\n        obj.before()\n        obj.action()\n",
        "legacy_linear_call_footer.py",
        &LEVELS,
    );
}

#[test]
fn empty_conditionals_preserve_legacy_terminal_return_locations() {
    for (case, source) in [
        "def f(a, b, obj):\n    if a:\n        assert a\n    if b:\n        obj.action()\n",
        "def f(a, b, obj):\n    if a:\n        global marker\n    if b:\n        obj.action()\n",
        "def f(a, b, obj):\n    if a:\n        assert a\n    obj.action()\n    if b:\n        obj.action()\n",
        "def f(a, b, obj):\n    if a:\n        assert a\n    if b:\n        assert b\n    else:\n        value = obj.action()\n    return obj if b else a\n",
        "def f(a, b, obj):\n    if a:\n        pass\n    if b:\n        obj.action()\n",
        "def f(a, b, obj):\n    if a:\n        assert a\n    if b:\n        value = obj.action()\n",
        "def f(a, b, c, obj):\n    if a:\n        assert a\n    if b:\n        if c:\n            value = obj.attr\n",
        "def f(a, b, obj):\n    if b:\n        obj.action()\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("empty_condition_terminal_return_{case}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn terminal_loop_footer_certificate_excludes_nested_conditionals() {
    for (name, source) in [
        (
            "nested_pass",
            "def f(a, b, c):\n    if a:\n        assert a\n    if b:\n        if c:\n            pass\n",
        ),
        (
            "nested_attribute",
            "def f(a, b, c, obj):\n    if a:\n        assert a\n    if b:\n        if c:\n            value = obj.attr\n",
        ),
        (
            "linear_loop",
            include_str!(
                "fixtures/optimization/3.12+3.13+3.14+3.15/0+1+2/native_terminal_if_footer_provenance/linear/live_extra.py"
            ),
        ),
        (
            "conditional_loop",
            include_str!(
                "fixtures/optimization/3.12+3.13+3.14+3.15/0+1+2/native_terminal_if_footer_provenance/conditional/erased_extra.py"
            ),
        ),
    ] {
        assert_matches_cpython(
            source,
            &format!("terminal_loop_footer_certificate_{name}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn consecutive_elided_conditions_preserve_empty_branch_locations() {
    for (prefix, suffix) in [
        (
            "    if a:\n        assert a\n",
            "    if c:\n        assert c\n",
        ),
        ("", "    if c:\n        assert c\n"),
        ("    if a:\n        assert a\n", "    action()\n"),
    ] {
        let source = format!(
            "def f(a, b, c):\n{prefix}    if b:\n        assert b\n    else:\n        assert c\n{suffix}"
        );
        assert_matches_cpython(&source, "consecutive_elided_conditions.py", &LEVELS);
    }
}

#[test]
fn protected_coroutine_branches_keep_returns_before_erased_assertions() {
    for (case, source) in [
        "async def f(a, b):\n    try:\n        if a:\n            assert a\n        assert b\n    except ValueError:\n        pass\n",
        "async def f(a, b):\n    try:\n        if a:\n            pass\n        assert b\n    except ValueError:\n        pass\n",
        "async def f(a, b):\n    try:\n        if a:\n            action()\n        assert b\n    except ValueError:\n        pass\n",
        "async def f(a, b):\n    try:\n        if a:\n            value = b\n        assert b\n    except ValueError:\n        pass\n",
        "async def f(a):\n    try:\n        value = await action()\n        if a is not None:\n            assert value\n        assert a\n    except ValueError:\n        if a is not None:\n            assert a\n        else:\n            raise\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("protected_coroutine_elided_branch_{case}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn legacy_protected_and_coroutine_branches_keep_returns_before_erased_assertions() {
    for (case, source) in [
        "def f(a):\n    try:\n        if a:\n            pass\n        assert a\n    except:\n        raise\n",
        "def f(a):\n    try:\n        if a:\n            action()\n        assert a\n    except ValueError:\n        recover()\n        raise\n",
        "async def f(a, b):\n    if a:\n        if b:\n            pass\n        assert a\n",
        "async def f(a, b):\n    if a:\n        if b:\n            action()\n        assert a\n",
        "async def f(a, b):\n    await action()\n    if a:\n        if b:\n            value = a\n        assert a\n",
        "async def f(a):\n    try:\n        await action()\n        if a:\n            pass\n        assert a\n    except ValueError:\n        raise\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("legacy_elided_branch_{case}.py"),
            &LEVELS,
            &[PythonVersion::Py313],
        );
    }
}

#[test]
fn coroutine_nested_branches_keep_returns_before_erased_assertions() {
    for (case, source) in [
        "async def f(a, b):\n    if a:\n        if b:\n            pass\n        assert a\n",
        "async def f(a, b):\n    if a:\n        if b:\n            value = action()\n        assert a\n",
        "async def f(a, b):\n    if a:\n        assert a\n    else:\n        if b:\n            action()\n        assert b\n",
        "async def f(a, b):\n    await action()\n    if a:\n        if b:\n            value = a\n        assert a\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("coroutine_nested_return_{case}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn coroutine_linear_tails_keep_returns_before_erased_assertions() {
    for (case, source) in [
        "async def f(a, b, c):\n    if a:\n        return\n    else:\n        if b:\n            if c:\n                action()\n            action()\n        assert b\n",
        "async def f(a, b, items):\n    if a:\n        if b:\n            value = [item for item in items]\n        assert a\n",
        "async def f(a, b, c, items):\n    if a:\n        return\n    else:\n        if b:\n            value = [item for item in items]\n            if c:\n                action()\n            value = a + b\n        assert b\n",
        "async def f(a, b, c, manager):\n    async with manager:\n        await action()\n    if a:\n        return\n    else:\n        if b:\n            if c:\n                action()\n            action()\n        assert b\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("coroutine_linear_tail_{case}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn removed_assertions_preserve_constant_loop_break_borrow_boundary() {
    for (index, body) in [
        "value = obj.next()\nif value:\n    break\nassert value\n",
        "value = obj.next()\naction()\nif value in (1, 2):\n    break\nassert value\n",
        "value = obj.next()\nif value:\n    break\nassert value\nassert obj\n",
    ]
    .into_iter()
    .enumerate()
    {
        let mut source = String::from("def f(obj):\n    while True:\n");
        for line in body.lines() {
            source.push_str("        ");
            source.push_str(line);
            source.push('\n');
        }
        source.push_str("    obj.close()\n");
        assert_matches_cpython(&source, &format!("constant_loop_break_{index}.py"), &LEVELS);
    }
}

#[test]
fn empty_try_before_removed_assertions_preserves_implicit_return() {
    for (case, source) in [
        "def f(flag):\n    try:\n        assert flag\n    except AssertionError:\n        pass\n    assert flag\n",
        "def f(flag):\n    try:\n        assert flag\n        assert flag, describe()\n    except AssertionError as error:\n        pass\n    assert flag\n    assert flag\n",
        "def f(flag):\n    \"description\"\n    action()\n    try: assert flag\n    except: pass\n    assert flag; assert flag\n",
        "def f(flag):\n    try:\n        assert flag\n    except AssertionError:\n        recover()\n    assert flag\n",
        "def f(flag):\n    try:\n        assert flag\n    except AssertionError:\n        return 1\n    assert flag\n",
        "def f(flag):\n    try:\n        assert flag\n    except AssertionError:\n        pass\n    assert flag\n    action()\n",
        "def f(flag):\n    try:\n        assert flag\n        pass\n    except AssertionError:\n        pass\n    assert flag\n",
        "def f(flag):\n    try:\n        assert flag\n    except AssertionError:\n        pass\n    assert flag\n    pass\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("empty_try_assert_tail_{case}.py"), &LEVELS);
    }
}

#[test]
fn elided_terminal_boolean_preserves_shared_return_order() {
    for test in [
        "a or b",
        "a == 1 or b == 2",
        "a or b or c",
        "a and b",
        "a and (b or c)",
        "not a and (b or c)",
        "(a or b) and (c or d)",
        "not a and (not b or c)",
        "not a.ready and (b() or c.ready)",
    ] {
        let source = format!("def f(a, b, c, d):\n    if {test}:\n        assert a\n");
        assert_matches_cpython(&source, "elided_terminal_boolean.py", &LEVELS);
    }
    for (body, suffix) in [
        ("pass", ""),
        ("consume(a)", ""),
        ("assert a\n        consume(a)", ""),
        ("assert a", "    consume(a)\n"),
    ] {
        let source =
            format!("def f(a, b, c):\n    if not a and (b or c):\n        {body}\n{suffix}");
        assert_matches_cpython(&source, "terminal_boolean_live_body.py", &LEVELS);
    }
}

#[test]
fn erased_try_arms_preserve_large_tuple_return_join() {
    for returned in [
        "value,",
        "value, owner",
        "1, 2",
        "1, 2, 3",
        "value, owner, first",
        "value, owner, first, {\"first\": first, \"second\": second}",
    ] {
        let source = format!(
            "def f(flag, first, second):\n    owner = acquire()\n    try:\n        value = owner.value\n        if flag:\n            assert ready(value)\n        else:\n            assert valid(value)\n        assert complete(value)\n        return {returned}\n    except Exception:\n        owner.close()\n        raise\n"
        );
        assert_matches_cpython(&source, "erased_try_tuple_return.py", &LEVELS);
    }
    assert_matches_cpython(
        "def f(flag, first, second):\n    if flag:\n        assert first\n    else:\n        assert second\n    assert flag\n    return first, second\n",
        "erased_unprotected_tuple_return.py",
        &LEVELS,
    );
}

#[test]
fn erased_assertions_preserve_terminal_protected_join_locations() {
    let source = include_str!("fixtures/protected_join_footers.py");
    for source in [
        source.to_owned(),
        source.replace("assert value", "consume(value)"),
        source.replace("        assert value\n", ""),
    ] {
        assert_matches_cpython(&source, "protected_join_footers.py", &LEVELS);
    }
}

#[test]
fn erased_assertions_preserve_try_body_joins_and_false_edge_locations() {
    for function in ["def", "async def"] {
        for alternate in [
            "chosen = converted",
            "if value:\n                return converted",
            "if value:\n                raise Problem(value)",
            "if value:\n                chosen = converted\n            else:\n                chosen = value",
        ] {
            let source = format!(
                "{function} f(value):\n    try:\n        converted = convert(value)\n        if predicate(converted):\n            chosen = converted[0]\n        else:\n            {alternate}\n        assert chosen\n    except Exception as error:\n        raise Problem(value) from error\n    consume(chosen)\n"
            );
            assert_matches_cpython(&source, "erased_try_body_join.py", &LEVELS);
        }
    }
}

#[test]
fn elided_try_guards_keep_python313_false_edge_return_locations() {
    for (index, source) in [
        "def f(a):\n    try:\n        if a:\n            return\n        assert False\n    except:\n        pass\n",
        "def f(a):\n    try:\n        if a:\n            action()\n            raise ValueError\n        assert False\n    except ValueError:\n        raise\n",
        "async def f(a):\n    try:\n        if a:\n            return await action()\n        assert False\n    except ValueError:\n        raise\n",
        "async def f(a, b):\n    try:\n        if a:\n            return await first()\n        elif b:\n            value = await second()\n            consume(value)\n            return value\n        assert False\n    except ValueError:\n        raise\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("python313_elided_try_guard_{index}.py"),
            &LEVELS,
            &[PythonVersion::Py313],
        );
    }
}

#[test]
fn elided_coroutine_try_guards_preserve_false_edge_return_locations() {
    const SOURCE: &str = "async def f(a, b, obj):\n    try:\n        if a is not None:\n            return await obj.first(a)\n        elif b is not None:\n            value = await obj.second(b)\n            obj.check(value)\n            return obj.wrap(value)\n        assert a or b\n    except Error as exc:\n        raise ValueError from exc\n    except Exception as exc:\n        obj.log(exc)\n        raise ValueError from exc\n";
    for (index, source) in [
        SOURCE.to_owned(),
        SOURCE.replace("await ", ""),
        SOURCE.replace("return obj.wrap(value)", "raise ValueError(value)"),
        SOURCE
            .replace("return await obj.first(a)", "raise ValueError(a)")
            .replace("return obj.wrap(value)", "raise TypeError(value)"),
        SOURCE.replace("    try:\n", "    try:\n        if obj.ready:\n            obj.prepare()\n"),
        SOURCE.replace("        assert a or b\n", "        assert a or b\n        assert obj\n"),
        "async def f(a, obj):\n    try:\n        if a is not None:\n            return await obj.first(a)\n        assert a\n    except Error:\n        raise\n".to_owned(),
        SOURCE.replace("        assert a or b\n", ""),
        SOURCE.replace("        assert a or b\n", "        assert a or b\n        obj.finish()\n"),
        SOURCE.replace("        assert a or b\n", "        else:\n            return None\n        assert a or b\n"),
        SOURCE.replace("async def", "def").replace("await ", ""),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("elided_coroutine_try_guard_{index}.py"), &LEVELS);
    }
}

#[test]
fn elided_protected_elif_preserves_loop_condition_owner() {
    for (function, manager) in [
        ("def", "with"),
        ("async def", "with"),
        ("async def", "async with"),
    ] {
        let source = format!(
            "{function} f(cm, items, a, b):
    {manager} cm:
        for item in items:
            if a:
                assert item
            elif b:
                assert items
            else:
                assert a
"
        );
        assert_matches_cpython(&source, "elided_protected_elif.py", &LEVELS);
    }
}

#[test]
fn with_conditional_exit_before_removed_assertion() {
    for (index, body) in [
        "if flag:\n    return 1\nassert flag\n",
        "if flag:\n    raise ValueError\nassert flag\n",
        "if flag is None:\n    return 1\nassert flag\nassert other\n",
        "if flag:\n    return 1\nelif other:\n    return 2\nassert flag\n",
        "if flag:\n    return 1\nelse:\n    return 2\nassert flag\n",
        "work()\nif flag:\n    return 1\nassert flag\n",
        "if flag:\n    work()\nassert flag\n",
    ]
    .into_iter()
    .enumerate()
    {
        let mut source = String::from("def f(cm, flag, other, work):\n    with cm:\n");
        for line in body.lines() {
            source.push_str("        ");
            source.push_str(line);
            source.push('\n');
        }
        assert_matches_cpython(
            &source,
            &format!("with_conditional_exit_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn nested_assignment_exits_survive_removed_assertions() {
    for (index, source) in [
        "def f(a,b,c):\n    if a:\n        value=1\n    else:\n        if b:\n            if c:\n                value=2\n            else:\n                value=3\n        else:\n            value=4\n        assert value\n",
        "def f(a,b,c):\n    if a:\n        value = choose(1)\n        assert value\n    else:\n        value = choose(2)\n        if b:\n            value = choose(3)\n            if a == 1:\n                value = choose(4)\n            elif a == 2:\n                value = choose(5)\n            else:\n                value = choose(6)\n        else:\n            value = choose(7)\n        assert value\n",
        "def f(a,b,c):\n    value = choose(0)\n    assert value\n    if a:\n        value = 1\n        assert value\n    else:\n        value = 2\n        if b and c:\n            value = 3\n            if a == 1:\n                value = 4\n            elif a == 2:\n                value = 5\n            else:\n                value = 6\n        else:\n            value = 7\n        assert value\n",
        "def f(a,b,c,obj,items):\n    if a:\n        value=1\n    else:\n        if b:\n            if c: value=2\n            else: value=3\n        else:\n            value=4\n        assert value\n",
        "def f(a,b,c):\n    if a:\n        value=1\n    else:\n        if b:\n            if c:\n                value=2\n            else:\n                value=3\n        else:\n            value=4\n",
        "def f(a,b):\n    if a:\n        if b:\n            value=1\n        else:\n            value=2\n    else:\n        value=3\n    assert value\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("nested_assignment_exit_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn nested_linear_branch_preserves_returns_before_removed_assertions() {
    for (index, leaf) in [
        "value = action()",
        "obj.value = action()",
        "value, other = action()",
        "action()",
        "b += 1",
        "assert b",
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!(
            "def f(a, b, obj):\n    if a:\n        if b:\n            {leaf}\n        assert a\n"
        );
        assert_matches_cpython(
            &source,
            &format!("nested_linear_return_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn nested_branch_returns_survive_removed_inner_assertions() {
    for (index, source) in [
        "def f(a, b):\n    if a:\n        if b:\n            pass\n            assert b\n        assert a\n",
        "def f(a, b):\n    if a:\n        if b:\n            value = action()\n            assert b\n        assert a\n",
        "def f(a, b, c):\n    if a:\n        if b:\n            if c:\n                action()\n            action()\n        assert a\n",
        "def f(a, b, c):\n    if a:\n        action()\n    else:\n        if b:\n            if c:\n                value = b\n            value = action()\n        assert a\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("nested_branch_assert_return_{index}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn nested_authored_pass_preserves_returns_before_removed_assertions() {
    for (index, source) in [
        "def f(a, b, c):\n    if a:\n        if b:\n            if c:\n                pass\n        assert a\n",
        "def f(a, b, c):\n    if a:\n        pass\n    else:\n        if b:\n            if c:\n                pass\n        assert a\n        assert b\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("nested_authored_pass_return_{index}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn terminal_else_preserves_nested_conditional_exits_before_removed_assertions() {
    for (index, leaf) in ["assert marker", "pass", "marker = first"]
        .into_iter()
        .enumerate()
    {
        let source = format!(
            "def f(first, second):
    marker = setup()
    if first is None:
        assert marker
    else:
        if second is None:
            {leaf}
        assert marker
"
        );
        assert_matches_cpython(&source, &format!("terminal_else_{index}.py"), &LEVELS);
    }
}

#[test]
fn erased_assertion_preserves_conditional_break_backedge_donor() {
    let source = r#"def select(groups, expected):
    for group in groups:
        valid = True
        for field in group:
            if field.kind == "main":
                value = decode(field)
                if value != expected:
                    report(value)
                    valid = False
                    break
                assert value
            elif field.kind == "extra":
                if ready(field) and valid and expected:
                    details = decode(field)
                    if details not in known:
                        known[details] = {}
                    value = known[details]
        if valid:
            yield group
"#;
    for source in [
        source.to_owned(),
        source.replace("yield group", "consume(group)"),
        source.replace("                assert value\n", ""),
        source.replace(
            "if value != expected:",
            "if value != expected or invalid(field):",
        ),
        source.replace(
            "if value != expected:",
            "if (\n                    value\n                    != expected\n                ):",
        ),
        source.replace(
            "                    report(value)\n                    valid = False\n",
            "",
        ),
        source.replace("assert value", "consume(value)"),
    ] {
        assert_matches_cpython(&source, "conditional_break_backedge_donor.py", &LEVELS);
    }
}

#[test]
fn erased_assertion_preserves_try_break_exit_donor() {
    let source = "async def f(manager):\n    async with manager:\n        while True:\n            try:\n                await work()\n                value = await read()\n                state = value[\"state\"]\n                if state in (\"ready\", \"failed\"):\n                    break\n                assert state in (\"waiting\", \"busy\")\n            except Error:\n                recover()\n        await finish()\n    return True\n";
    for source in [
        source.to_owned(),
        source.replace(
            "                assert state in (\"waiting\", \"busy\")\n",
            "",
        ),
        source.replace("                recover()", "                pass"),
    ] {
        assert_matches_cpython(&source, "try_break_exit_donor.py", &LEVELS);
    }
}

#[test]
fn elided_assertions_preserve_terminal_loop_break_locations() {
    for (case, source) in [
        "def f(flag, items):\n    if flag:\n        for item in items:\n            if item:\n                break\n        assert flag\n",
        "def f(flag, items):\n    if flag:\n        action()\n    else:\n        for item in items:\n            if item:\n                break\n        assert flag\n        assert items\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("terminal_loop_break_{case}.py"), &LEVELS);
    }
}

#[test]
fn elided_assertions_preserve_linear_assignment_return_boundary() {
    let source = "def check(first, second):\n    if first:\n        if second:\n            left()\n        else:\n            right()\n        value = \"text\"\n        assert value\n    else:\n        other()\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("value = \"text\"", "value = second"),
        source.replace("value = \"text\"", "value = choose()"),
        source.replace("value = \"text\"", "value = \"left\" if second else \"right\""),
        source.replace("value = \"text\"", "value = first or second"),
        source.replace("        assert value\n", ""),
        source.replace("        assert value\n", "        return\n"),
        "def check(first, second):\n    if first:\n        other()\n    else:\n        if second:\n            left()\n        else:\n            right()\n        value = \"text\"\n        assert value\n".to_owned(),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("linear_assignment_assert_{index}.py"), &LEVELS);
    }
}

#[test]
fn elided_assertions_preserve_conditional_assignment_return_boundary() {
    for (case, source) in [
        "def f(flag, value):\n    if value is not None:\n        result = 1 if flag else 2\n        assert result\n",
        "def f(flag, value):\n    if value is None:\n        consume()\n    else:\n        result = 1 if flag else 2\n        assert result\n        assert flag\n",
        "def f(obj, flag, value):\n    if value is not None:\n        obj.result = 1 if flag else 2\n        assert obj.result\n    else:\n        obj.result = None\n",
        "def f(obj, flag, value):\n    if value is not None:\n        obj[0] = 1 if flag else 2\n        assert flag\n",
        "def f(flag, value):\n    if value is None:\n        consume()\n    elif flag:\n        result = 1 if value else 2\n        assert result\n    else:\n        done()\n",
        "def f(flag):\n    result = 1 if flag else 2\n    assert result\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("conditional_assignment_assert_{case}.py"), &LEVELS);
    }
}

#[test]
fn elided_assertions_preserve_unpacking_return_locations() {
    for locals in [0, 16] {
        for targets in [
            "first, second",
            "first, second, third",
            "first, second, third, fourth",
        ] {
            for brackets in [false, true] {
                let mut source = String::from("def f(obj):\n");
                for index in 0..locals {
                    writeln!(source, "    temporary_{index} = {index}").unwrap();
                }
                let values = targets
                    .split(", ")
                    .map(|name| format!("obj.{name}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let target = if brackets {
                    format!("[{targets}]")
                } else {
                    targets.to_owned()
                };
                writeln!(source, "    {target} = {values}\n    assert first").unwrap();
                assert_matches_cpython(&source, "unpacking_before_assert.py", &LEVELS);
            }
        }
    }
}

#[test]
fn elided_assertions_preserve_assignment_before_raising_scope_exit() {
    for value in [
        "obj.value",
        "(1, 2, 3)",
        "(obj.first, obj.second, obj.third)",
        "[obj.first, obj.second]",
        "consume(obj)",
        "obj",
    ] {
        let source = format!(
            "def f(flag, value, obj):\n    if flag:\n        assert flag\n    if value:\n        values = {value}\n        if flag not in values:\n            raise ValueError()\n        if flag:\n            raise ValueError()\n"
        );
        for source in [
            source.clone(),
            source.replace("        assert flag", "        consume(flag)"),
            format!("{source}    return None\n"),
        ] {
            assert_matches_cpython(&source, "assignment_before_raising_scope_exit.py", &LEVELS);
        }
    }
}

#[test]
fn elided_assertions_preserve_conditional_await_footer() {
    for prefix in [
        "",
        "    await consume()\n    if flag:\n        assert flag\n",
        "    if flag:\n        assert flag\n",
        "    if flag:\n        consume()\n",
        "    if flag:\n        assert flag\n    consume()\n",
    ] {
        for test in ["flag", "flag is None"] {
            for awaited in ["consume()", "value"] {
                let source = format!(
                    "async def f(flag, value):\n{prefix}    if {test}:\n        await {awaited}\n"
                );
                for source in [
                    source.clone(),
                    format!("{source}    return None\n"),
                    format!("{source}    else:\n        consume()\n"),
                ] {
                    assert_matches_cpython(&source, "conditional_await_footer.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn elided_assertions_preserve_conditional_unpacking_return_locations() {
    for scope in ["def", "async def"] {
        for targets in [
            "first, second",
            "first, second, third",
            "first, second, third, fourth",
        ] {
            let values = targets
                .split(", ")
                .map(|name| format!("obj.{name}"))
                .collect::<Vec<_>>()
                .join(", ");
            for brackets in [false, true] {
                let target = if brackets {
                    format!("[{targets}]")
                } else {
                    targets.to_owned()
                };
                for tail in [
                    "",
                    "        assert first\n",
                    "        assert first\n        assert flag\n",
                    "        return first\n",
                ] {
                    for otherwise in [false, true] {
                        let branch = if otherwise {
                            "    if flag:\n        consume()\n    else:\n"
                        } else {
                            "    if flag:\n"
                        };
                        let source = format!(
                            "{scope} f(obj, flag):\n{branch}        {target} = {values}\n{tail}"
                        );
                        assert_matches_cpython(
                            &source,
                            "conditional_unpacking_before_assert.py",
                            &LEVELS,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn elided_assertions_preserve_conditional_store_return_locations() {
    for assignment in [
        "value = consume()",
        "first = second = consume()",
        "obj.value = consume()",
        "obj[0] = consume()",
        "obj.value, value = consume(), consume()",
        "obj[0], value = consume(), consume()",
    ] {
        let source =
            format!("def f(obj, flag):\n    if flag:\n        {assignment}\n        assert flag\n");
        assert_matches_cpython(&source, "conditional_store_before_assert.py", &LEVELS);
    }
}

#[test]
fn while_else_keeps_finally_borrowing_after_elided_assertions() {
    for condition in ["condition()", "condition() or count < 3"] {
        for following in ["", "    return obj\n"] {
            let source = format!(
                "def f(obj, count):
    try:
        while {condition}:
            action()
        else:
            recover()
        assert obj
    except ValueError:
        recover()
    finally:
        obj.close()
{following}"
            );
            assert_matches_cpython_versions(
                &source,
                "while_else_finally_borrow.py",
                &LEVELS,
                &[
                    PythonVersion::Py313,
                    PythonVersion::Py314,
                    PythonVersion::Py315,
                ],
            );
        }
    }
}

#[test]
fn terminal_chained_comparisons_preserve_empty_branch_exits() {
    for (index, body) in [
        "assert value",
        "global declared",
        "global declared\n        assert value",
    ]
    .into_iter()
    .enumerate()
    {
        for (name, prefix, keyword, suffix) in [
            ("plain", "", "if", ""),
            ("else", "", "if", "    else:\n        raise ValueError\n"),
            (
                "elif",
                "    if value is None:\n        raise ValueError\n",
                "elif",
                "    else:\n        raise ValueError\n",
            ),
        ] {
            let source = format!(
                "def f(value):\n{prefix}    {keyword} 0 < value < 3:\n        {body}\n{suffix}"
            );
            assert_matches_cpython(
                &source,
                &format!("empty_comparison_{name}_{index}.py"),
                &LEVELS,
            );
        }
    }
    assert_matches_cpython(
        "def f(low, value, high, other):
    if low < value < high:
        global state
    else:
        if low < other < high:
            global seen
",
        "nested_empty_comparison.py",
        &LEVELS,
    );
}

#[test]
fn elided_assertions_preserve_protected_while_exit_location() {
    for (case, condition) in ["condition()", "condition() or count < 3"]
        .into_iter()
        .enumerate()
    {
        let source = format!(
            "def f(obj, count):\n    try:\n        while {condition}:\n            action()\n        assert obj\n    except ValueError:\n        recover()\n    finally:\n        obj.close()\n"
        );
        assert_matches_cpython(&source, &format!("protected_while_exit_{case}.py"), &LEVELS);
    }
}

#[test]
fn elided_assertion_after_simple_if_preserves_finally_borrow_boundary() {
    for (case, branches) in [
        "",
        "        else:\n            pass\n",
        "        elif other():\n            recover()\n",
        "        elif other():\n            recover()\n        else:\n            pass\n",
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!(
            "def f(obj, count):
    try:
        if condition():
            action()
{branches}        assert obj
    except ValueError:
        recover()
    finally:
        obj.close()
"
        );
        assert_matches_cpython(
            &source,
            &format!("simple_if_finally_borrow_{case}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn discarded_imports_preserve_none_admission_order() {
    for (index, import) in [
        "import package",
        "import package.module as alias",
        "import package, other",
        "from package import item",
    ]
    .into_iter()
    .enumerate()
    {
        for (scope, prefix, indent) in [
            ("module", "", ""),
            ("function", "def f():\n", "    "),
            ("class", "class Outer:\n", "    "),
        ] {
            let source = format!(
                "{prefix}{indent}if False:\n{indent}    {import}\n\
                 {indent}class Later:\n{indent}    pass\n"
            );
            assert_matches_cpython(
                &source,
                &format!("discarded_import_{index}_{scope}.py"),
                &LEVELS,
            );
        }
    }
}

#[test]
fn dotted_import_parser_interning_matches_cpython() {
    for (case, import) in [
        "if False:\n    from SercOuter.SercInner import item\n",
        "if False:\n    from SercOuter.SercInner.Leaf import item\n",
        "seed = None\nif False:\n    from SercOuter.SercInner.Leaf import *\n",
        "seed = None\nif False:\n    import SercOuter.SercInner.Leaf\n",
        "from SercOuter.SercInner import item\n",
        "if condition:\n    from SercOuter.SercInner import item\n",
        "if False:\n    from SercOuter import item\n",
        "if False:\n    from SercOuter.SercInnerExtra.Leaf import item\n",
        "from SercOuter.SercInner.Leaf import item\nsaved = {'SercOuter.SercInner', 'other'}\n",
        "if False:\n    from SercOuter.SercInner.Leaf import item\nfirst = ('SercOuter.SercInner', ('SercOuter.SercInner',))\nsecond = ('SercOuter.' + 'SercInner', ('SercOuter.SercInner',))\n",
        "def select(flag):\n    try:\n        if True:\n            if flag:\n                return 1\n        else:\n            from SercOuter.SercInner.Leaf import item\n    except Exception:\n        return None\n",
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!("{import}class SercOuter:\n    class SercInner:\n        pass\n");
        assert_matches_cpython(&source, &format!("dotted_import_{case}.py"), &LEVELS);
    }
    for (case, import) in [
        "from A.B.C.Leaf import item",
        "from A.B.C.Leaf import *",
        "import A.B.C.Leaf",
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!(
            "seed = None\nif False:\n    {import}\nclass A:\n    class B:\n        class C:\n            pass\n"
        );
        assert_matches_cpython(&source, &format!("deep_dotted_import_{case}.py"), &LEVELS);
    }
    let source = "if False:\n    from \u{fb00}.SercInner.Leaf import item\nclass ff:\n    class SercInner:\n        pass\n";
    assert_matches_cpython(source, "dotted_import_nfkc.py", &LEVELS);
    let source = "if False:\n    from SercOuter.__SercInner.Leaf import item\nclass SercOuter:\n    class __SercInner:\n        pass\n";
    assert_matches_cpython(source, "dotted_import_private.py", &LEVELS);
}

#[test]
fn discarded_star_imports_match_cpython() {
    for (case, source) in [
        "if False:\n    from package import *\nvalue = 1\n",
        "if False:\n    from package.child import *\nvalue = 1\n",
        "if False:\n    from . import *\nvalue = 1\n",
        "if False:\n    from . import name\nvalue = 1\n",
        "while False:\n    from package import *\nvalue = 1\n",
        "if True:\n    value = 1\nelse:\n    from package import *\n",
        "try:\n    if False:\n        from package.child import *\n    else:\n        value = 1\nexcept Exception:\n    value = 2\n",
        "try:\n    if True:\n        value = 1\n    else:\n        from . import name\nexcept Exception:\n    value = 2\n",
        "if enabled:\n    from package import *\nvalue = 1\n",
        "from package import *\nvalue = 1\n",
        "if False:\n    from package import name\nvalue = 1\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("star_import_{case}.py"), &LEVELS);
    }
}

#[test]
fn named_handler_loop_cleanup_after_elided_assertions_matches_cpython() {
    for (index, suffix) in [
        "assert error\n",
        "assert error\n        assert items\n",
        "pass\n        assert error\n",
        "assert error\n        pass\n",
        "pass\n",
        "",
    ]
    .into_iter()
    .enumerate()
    {
        for definition in ["def", "async def"] {
            let source = format!(
                "{definition} f(items):
    try:
        action()
    except ValueError as error:
        for item in items:
            if item:
                break
        {suffix}"
            );
            assert_matches_cpython(&source, &format!("named_handler_loop_{index}.py"), &LEVELS);
        }
    }
}

#[test]
fn exceptional_finally_return_unwinds_managers_in_order() {
    const FINALLY: &str = r"def f(outer, inner, work, cleanup):
    with outer:
        try:
            work()
        finally:
            with inner:
                try:
                    return work()
                finally:
                    cleanup()
";
    const HANDLER: &str = r"def f(outer, inner, work, cleanup):
    with outer:
        try:
            work()
        finally:
            with inner:
                try:
                    return work()
                except ValueError:
                    return 'handled'
";
    const LOOP_OUTSIDE: &str = r"def f(outer, inner, work, cleanup):
    with outer:
        try:
            work()
        finally:
            for x in (1,):
                with inner:
                    return work()
";
    const LOOP_INSIDE: &str = r"def f(outer, inner, work, cleanup):
    with outer:
        try:
            work()
        finally:
            with inner:
                for x in (1,):
                    return work()
";
    const MANAGER_IN_HANDLER: &str = r"def f(outer, inner, work, cleanup):
    with outer:
        try:
            work()
        finally:
            try:
                work()
            except ValueError:
                with inner:
                    return work()
";
    const MANAGER_IN_HANDLER_IN_LOOP: &str = r"def f(outer, inner, work, cleanup):
    with outer:
        try:
            work()
        finally:
            for x in (1,):
                try:
                    work()
                except ValueError:
                    with inner:
                        return work()
";
    const TRACE: &str = r"
import itertools, json, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
class WorkFault(Exception): pass
class CleanupFault(Exception): pass
class ExitFault(Exception): pass
def chain(error):
    seen, result = set(), []
    while error is not None and id(error) not in seen:
        seen.add(id(error))
        result.append(type(error).__name__)
        error = error.__context__
    return result
rows = []
for first, second, cleanup_raises, inner_mode, outer_mode in itertools.product(
        (False, True), ('none', 'work', 'value'), (False, True),
        ('normal', 'suppress', 'raise'), ('normal', 'suppress', 'raise')):
    events = []
    class Manager:
        def __init__(self, name, mode):
            self.name, self.mode = name, mode
        def __enter__(self):
            events.append(('enter', self.name))
            return self
        def __exit__(self, typ, val, tb):
            events.append(('exit', self.name, typ.__name__ if typ else None, chain(val)))
            if self.mode == 'raise':
                raise ExitFault(self.name)
            return self.mode == 'suppress'
    calls = [0]
    def work():
        calls[0] += 1
        events.append(('work', calls[0]))
        if calls[0] == 1 and first:
            raise WorkFault('first')
        if calls[0] == 2 and second == 'work':
            raise WorkFault('second')
        if calls[0] == 2 and second == 'value':
            raise ValueError('second')
        return calls[0]
    def cleanup():
        events.append(('cleanup',))
        if cleanup_raises:
            raise CleanupFault()
    try:
        outcome = ('return', scope['f'](Manager('outer', outer_mode),
                                        Manager('inner', inner_mode), work, cleanup))
    except BaseException as error:
        outcome = ('raise', chain(error))
    rows.append((events, outcome))
print(json.dumps(rows))
";

    for source in [
        FINALLY,
        HANDLER,
        LOOP_OUTSIDE,
        LOOP_INSIDE,
        MANAGER_IN_HANDLER,
        MANAGER_IN_HANDLER_IN_LOOP,
    ] {
        for (index, (python_version, _, _)) in VERSIONS.iter().copied().enumerate() {
            for optimization_level in LEVELS {
                let [native, actual] = compile_both(
                    source,
                    "return_finally.py",
                    python_version,
                    optimization_level,
                );
                assert_eq!(actual, native, "{python_version}, {optimization_level:?}");
                assert_outputs_match(
                    probe(index).args(["-I", "-S", "-B", "-c", TRACE]),
                    [&native, &actual],
                );
            }
        }
    }
}

#[test]
fn exceptional_finally_return_inside_older_handler() {
    const GENERATOR: &str = r"def f(manager, work):
    try:
        yield 'ready'
    except ValueError:
        try:
            work()
        finally:
            with manager:
                return 7

def ordinary(manager, work):
    try:
        raise ValueError('trigger')
    except ValueError:
        try:
            work()
        finally:
            with manager:
                return 7
";
    const ASYNC: &str = r"async def f(outer, inner, work):
    async with outer:
        try:
            await work()
        except ValueError:
            try:
                await work()
            finally:
                async with inner:
                    return 7
";
    const GENERATOR_TRACE: &str = r"
import itertools, json, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
def chain(error):
    seen, result = set(), []
    while error is not None and id(error) not in seen:
        seen.add(id(error))
        result.append(type(error).__name__)
        error = error.__context__
    return result
rows = []
for action, work_fails, mode in itertools.product(
        ('resume', 'throw', 'close', 'ordinary'), (False, True),
        ('normal', 'suppress', 'raise')):
    events = []
    class Manager:
        def __enter__(self):
            events.append(('enter', type(sys.exception()).__name__))
            return self
        def __exit__(self, typ, val, tb):
            events.append(('exit', typ.__name__ if typ else None, chain(val)))
            if mode == 'raise':
                raise RuntimeError('exit')
            return mode == 'suppress'
    def work():
        events.append(('work', type(sys.exception()).__name__))
        if work_fails:
            raise ValueError('work')
    try:
        if action == 'ordinary':
            value = scope['ordinary'](Manager(), work)
        else:
            generator = scope['f'](Manager(), work)
            events.append(('first', next(generator)))
            if action == 'throw':
                value = generator.throw(ValueError('trigger'))
            elif action == 'close':
                value = generator.close()
            else:
                value = next(generator)
        outcome = ('value', value)
    except StopIteration as error:
        outcome = ('return', error.value, chain(error))
    except BaseException as error:
        outcome = ('raise', chain(error))
    rows.append((events, outcome))
print(json.dumps(rows))
";
    const ASYNC_TRACE: &str = r"
import asyncio, itertools, json, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
def chain(error):
    seen, result = set(), []
    while error is not None and id(error) not in seen:
        seen.add(id(error))
        result.append(type(error).__name__)
        error = error.__context__
    return result
rows = []
for failures, action, mode in itertools.product(
        (1, 2), ('resume', 'throw', 'cancel', 'close'), ('normal', 'suppress', 'raise')):
    events, steps, calls = [], [], [0]
    class Pause:
        def __await__(self):
            yield 'suspended'
    class Manager:
        def __init__(self, name):
            self.name = name
        async def __aenter__(self):
            events.append(('enter', self.name))
            return self
        async def __aexit__(self, typ, val, tb):
            events.append(('exit', self.name, typ.__name__ if typ else None, chain(val)))
            if self.name == 'inner':
                await Pause()
                if mode == 'raise':
                    raise RuntimeError('exit')
            return self.name == 'inner' and mode == 'suppress'
    async def work():
        calls[0] += 1
        events.append(('work', calls[0]))
        if calls[0] <= failures:
            raise ValueError('work')
    coroutine = scope['f'](Manager('outer'), Manager('inner'), work)
    try:
        first = coroutine.send(None)
        steps.append(first)
        if first != 'suspended':
            raise RuntimeError('inner exit did not suspend')
        if action == 'throw':
            value = coroutine.throw(RuntimeError('throw'))
        elif action == 'cancel':
            value = coroutine.throw(asyncio.CancelledError('cancel'))
        elif action == 'close':
            value = coroutine.close()
        else:
            value = coroutine.send(None)
        outcome = ('value', value)
    except StopIteration as error:
        outcome = ('return', error.value)
    except BaseException as error:
        outcome = ('raise', chain(error))
    rows.append((events, steps, outcome))
print(json.dumps(rows))
";

    for (source, trace) in [(GENERATOR, GENERATOR_TRACE), (ASYNC, ASYNC_TRACE)] {
        for (index, (python_version, _, _)) in VERSIONS.iter().copied().enumerate() {
            for optimization_level in LEVELS {
                let [native, actual] = compile_both(
                    source,
                    "return_older_handler.py",
                    python_version,
                    optimization_level,
                );
                assert_eq!(actual, native, "{python_version}, {optimization_level:?}");
                assert_outputs_match(
                    probe(index).args(["-I", "-S", "-B", "-c", trace]),
                    [&native, &actual],
                );
            }
        }
    }
}

#[test]
fn optimized_footer_keeps_protected_manager_pass_exception_owner_on_313() {
    const SOURCE: &str = r"def f(manager, x, y):
    try:
        with manager:
            pass
            work()
    except Exception:
        recover()
    if x:
        if y:
            pass
        assert y
";
    const TRACE: &str = r"
import dis, marshal, sys
events, hits = [], []
class Manager:
    def __enter__(self): events.append('enter')
    def __exit__(self, kind, value, traceback):
        events.append('exit:' + (kind.__name__ if kind else 'None'))
        return False
scope = {'work': lambda: events.append('work'), 'recover': lambda: events.append('recover')}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOP' and
           (i.positions.lineno, i.positions.end_lineno, i.positions.col_offset, i.positions.end_col_offset) == (4, 4, 12, 16)]
if len(targets) != 1: raise RuntimeError(('manager pass marker count', targets))
target = targets[0]
monitor = sys.monitoring
monitor.use_tool_id(5, 'manager-pass-optimized-footer')
def inject(code, offset):
    if offset == target:
        hits.append(offset)
        raise ValueError('injected')
monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
try:
    for x, y in [(False, False), (True, True)]:
        events.clear()
        hits.clear()
        result = f(Manager(), x, y)
        if hits != [target] or events != ['enter', 'exit:ValueError', 'recover'] or result is not None:
            raise RuntimeError((x, y, hits, events, result))
finally:
    monitor.set_local_events(5, f.__code__, 0)
    monitor.register_callback(5, monitor.events.INSTRUCTION, None)
    monitor.free_tool_id(5)
";
    for optimization_level in LEVELS {
        let [native, actual] = compile_both(
            SOURCE,
            "manager_footer.py",
            PythonVersion::Py313,
            optimization_level,
        );
        assert_eq!(actual, native, "{optimization_level:?}");
        assert_runs(
            probe(1)
                .args(["-I", "-S", "-B", "-c", TRACE])
                .stdout(Stdio::piped()),
            [&native, &actual],
        );
    }
}

#[test]
fn optimized_footer_keeps_split_manager_pass_exception_owner_on_313() {
    const SOURCE: &str = r"def f(manager, x, y):
    try:
        with manager:
            pass
            try:
                work()
            finally:
                cleanup()
    except Exception:
        recover()
    if x:
        if y:
            pass
        assert y
";
    const TRACE: &str = r"
import dis, marshal, sys
events, hits = [], []
class Manager:
    def __enter__(self): events.append('enter')
    def __exit__(self, kind, value, traceback):
        events.append('exit:' + (kind.__name__ if kind else 'None'))
        return False
scope = {'work': lambda: events.append('work'), 'cleanup': lambda: events.append('cleanup'), 'recover': lambda: events.append('recover')}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOP' and
           (i.positions.lineno, i.positions.end_lineno, i.positions.col_offset, i.positions.end_col_offset) == (4, 4, 12, 16)]
if len(targets) != 1: raise RuntimeError(('manager pass marker count', targets))
target = targets[0]
monitor = sys.monitoring
monitor.use_tool_id(5, 'manager-pass-optimized-footer')
def inject(code, offset):
    if offset == target:
        hits.append(offset)
        raise ValueError('injected')
monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
try:
    for x, y in [(False, False), (True, True)]:
        events.clear()
        hits.clear()
        result = f(Manager(), x, y)
        if hits != [target] or events != ['enter', 'exit:ValueError', 'recover'] or result is not None:
            raise RuntimeError((x, y, hits, events, result))
finally:
    monitor.set_local_events(5, f.__code__, 0)
    monitor.register_callback(5, monitor.events.INSTRUCTION, None)
    monitor.free_tool_id(5)
events.clear()
result = f(Manager(), False, False)
if events != ['enter', 'work', 'cleanup', 'exit:None'] or result is not None:
    raise RuntimeError((events, result))
";
    for optimization_level in LEVELS {
        let [native, actual] = compile_both(
            SOURCE,
            "manager_footer.py",
            PythonVersion::Py313,
            optimization_level,
        );
        assert_eq!(actual, native, "{optimization_level:?}");
        assert_runs(
            probe(1)
                .args(["-I", "-S", "-B", "-c", TRACE])
                .stdout(Stdio::piped()),
            [&native, &actual],
        );
    }
}

#[test]
fn optimized_footer_after_legacy_manager_finally_preserves_jump_owner() {
    const SOURCE: &str = r"def f(manager, work, x, y):
    try:
        raise ValueError()
    except ValueError:
        with manager:
            pass
            try:
                work()
            finally:
                pass
    if x:
        if y:
            pass
        assert y
";
    const TRACE: &str = r"
import dis, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'JUMP_FORWARD' and i.positions.lineno == 10]
if len(targets) != 1:
    raise RuntimeError(('missing finalizer jump', targets))
target = targets[0]
monitor = sys.monitoring
monitor.use_tool_id(5, 'legacy-manager-finally-footer')
try:
    for error_type in (RuntimeError, StopIteration):
        for suppress in (False, True):
            for x, y in ((False, False), (True, True)):
                events, hits = [], []
                class Manager:
                    def __enter__(self): events.append('enter')
                    def __exit__(self, typ, value, traceback):
                        events.append(('exit', typ))
                        return suppress
                def work(): events.append('work')
                def inject(code, offset):
                    if offset == target:
                        hits.append(offset)
                        raise error_type('injected')
                monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
                monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
                try:
                    result = f(Manager(), work, x, y)
                    outcome = None
                except BaseException as error:
                    result, outcome = None, type(error)
                finally:
                    monitor.set_local_events(5, f.__code__, 0)
                    monitor.register_callback(5, monitor.events.INSTRUCTION, None)
                expected = None if suppress else error_type
                if hits != [target] or events != ['enter', 'work', ('exit', error_type)] or outcome is not expected or result is not None:
                    raise RuntimeError((hits, events, outcome, result, expected))
finally:
    monitor.free_tool_id(5)
";
    for (index, python_version) in [(1, PythonVersion::Py313)] {
        for optimization_level in LEVELS {
            let [native, actual] = compile_both(
                SOURCE,
                "manager_finally_footer.py",
                python_version,
                optimization_level,
            );
            assert_eq!(actual, native, "{python_version}, {optimization_level:?}");
            assert_runs(
                probe(index).args(["-I", "-S", "-B", "-c", TRACE]),
                [&native, &actual],
            );
        }
    }
}

#[test]
fn optimized_footer_after_exceptional_manager_return_unwind() {
    const SOURCE: &str = r"def f(outer, inner, work, cleanup, events, take, x, y):
    with outer:
        try:
            work()
        finally:
            with inner:
                try:
                    if take:
                        return work()
                finally:
                    cleanup()
    events.append('footer')
    if x:
        if y:
            pass
        assert y
";
    const TRACE: &str = r"
import itertools, json, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
rows, covered = [], set()
class ExitFault(Exception): pass
for fault, take, suppress, pair, inner_raises, outer_raises in itertools.product((False, True), repeat=6):
    events, calls = [], [0]
    class Manager:
        def __init__(self, name): self.name = name
        def __enter__(self):
            events.append(('enter', self.name))
            return self
        def __exit__(self, typ, value, traceback):
            events.append(('exit', self.name, typ.__name__ if typ else None))
            if (self.name == 'inner' and inner_raises) or (self.name == 'outer' and outer_raises):
                events.append(('raise-exit', self.name))
                raise ExitFault(self.name)
            return suppress and self.name == 'outer'
    def work():
        calls[0] += 1
        events.append(('work', calls[0]))
        if fault and calls[0] == 1:
            raise ValueError('work')
        return 10 + calls[0]
    def cleanup(): events.append(('cleanup',))
    try:
        result = scope['f'](Manager('outer'), Manager('inner'), work, cleanup,
                            events, take, pair, pair)
        outcome = ('return', result)
    except BaseException as error:
        outcome = ('raise', type(error).__name__)
    if take and not inner_raises and not outer_raises and outcome != ('return', 12):
        raise RuntimeError(('return path not reached', events, outcome))
    if not take and not inner_raises and not outer_raises and (not fault or suppress) and 'footer' not in events:
        raise RuntimeError(('footer not reached', events, outcome))
    if ('enter', 'inner') not in events or ('cleanup',) not in events:
        raise RuntimeError(('finalizer not reached', events, outcome))
    if inner_raises and ('raise-exit', 'inner') not in events:
        raise RuntimeError(('inner exit not reached', events, outcome))
    if outer_raises and ('raise-exit', 'outer') not in events:
        raise RuntimeError(('outer exit not reached', events, outcome))
    if 'footer' in events: covered.add('footer')
    if take and outcome == ('return', 12): covered.add('return')
    if fault and suppress and 'footer' in events: covered.add('suppressed')
    if ('raise-exit', 'inner') in events: covered.add('inner_exit_fault')
    if ('raise-exit', 'outer') in events: covered.add('outer_exit_fault')
    rows.append((events, outcome))
if covered != {'footer', 'return', 'suppressed', 'inner_exit_fault', 'outer_exit_fault'}:
    raise RuntimeError(('missing coverage', covered))
print(json.dumps(rows))
";
    for optimization_level in LEVELS {
        let [native, actual] = compile_both(
            SOURCE,
            "return_manager_footer.py",
            PythonVersion::Py313,
            optimization_level,
        );
        assert_eq!(actual, native, "{optimization_level:?}");
        assert_outputs_match(
            probe(1).args(["-I", "-S", "-B", "-c", TRACE]),
            [&native, &actual],
        );
    }
}

#[test]
fn legacy_nested_finally_pass_keeps_outer_owner() {
    const SOURCE: &str = "def f(work, cleanup, after):\n    try:\n        try:\n            work()\n        finally:\n            pass\n        after()\n    finally:\n        cleanup()\n";
    const TERMINAL: &str = "def f(work, cleanup):\n    try:\n        try:\n            work()\n        finally:\n            pass\n    finally:\n        cleanup()\n";
    const TRACE: &str = r"
import dis, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [instruction.offset for instruction in dis.get_instructions(f)
           if instruction.opname == 'NOP' and instruction.positions.lineno == 6]
if not targets:
    raise RuntimeError('missing pass marker')
target = min(targets)
monitor = sys.monitoring
monitor.use_tool_id(5, 'legacy-finally-owner')
for exception in (ValueError, StopIteration):
    for cleanup_raises in (False, True):
        events, hits = [], []
        def work():
            events.append('work')
        def after():
            events.append('after')
        def cleanup():
            events.append('cleanup')
            if cleanup_raises:
                raise RuntimeError('cleanup')
        def inject(code, offset):
            if offset == target:
                hits.append(offset)
                raise exception('injected')
        monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
        monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
        try:
            f(work, cleanup, after)
            result = None
        except BaseException as error:
            result = type(error)
        finally:
            monitor.set_local_events(5, f.__code__, 0)
            monitor.register_callback(5, monitor.events.INSTRUCTION, None)
        expected = RuntimeError if cleanup_raises else exception
        if hits != [target] or events != ['work', 'cleanup'] or result is not expected:
            raise RuntimeError((hits, events, result, expected))
monitor.free_tool_id(5)
";

    let versions = [PythonVersion::Py312, PythonVersion::Py313];
    for source in [
        TERMINAL,
        r"def f(work,cleanup,flag):
    try:
        try:
            work()
        finally:
            pass
        global marker
        assert flag
    finally:
        cleanup()
",
        r"def outer():
    marker = 0
    def f(work,cleanup,flag):
        try:
            try:
                work()
            finally:
                pass
            nonlocal marker
            1
        finally:
            cleanup()
    return f
",
    ] {
        assert_matches_cpython_versions(source, "legacy_finally.py", &LEVELS, &versions);
    }
    for (index, python_version) in versions.into_iter().enumerate() {
        for optimization_level in LEVELS {
            let [native, actual] = compile_both(
                SOURCE,
                "legacy_finally.py",
                python_version,
                optimization_level,
            );
            assert_eq!(actual, native, "{python_version}, {optimization_level:?}");
            assert_runs(
                probe(index)
                    .args(["-I", "-S", "-B", "-c", TRACE])
                    .stdout(Stdio::piped()),
                [&native, &actual],
            );
        }
    }
}

#[test]
fn legacy_normal_finalizer_pass_keeps_outer_handler() {
    const TRACE: &str = r"
import dis, marshal, sys

events = []
def work():
    events.append('work')
def after():
    events.append('after')
def recover():
    events.append('recover')
scope = {'work': work, 'after': after, 'recover': recover}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [instruction.offset for instruction in dis.get_instructions(f)
           if instruction.opname == 'NOP' and instruction.positions.lineno == 10]
if not targets:
    raise RuntimeError('missing pass marker')
target = min(targets)
generator, has_sibling = (argument == '1' for argument in sys.argv[1:])
monitor = sys.monitoring
monitor.use_tool_id(5, 'legacy-normal-finalizer')
for fault in (ValueError, StopIteration):
    for flag in (False, True):
        events.clear()
        hits = []
        def inject(code, offset):
            if offset == target:
                hits.append(offset)
                raise fault('injected')
        monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
        monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
        try:
            value = f(None, flag)
            if generator:
                value = next(value)
            result = ('value', value)
        except BaseException as error:
            result = ('error', type(error))
        finally:
            monitor.set_local_events(5, f.__code__, 0)
            monitor.register_callback(5, monitor.events.INSTRUCTION, None)
        expected = ('value', 1) if has_sibling else (
            'error', RuntimeError if generator and fault is StopIteration else fault)
        expected_events = ['work', 'recover'] if has_sibling else ['work']
        if hits != [target] or events != expected_events or result != expected:
            raise RuntimeError((hits, events, result, expected))
monitor.free_tool_id(5)
";
    for generator in [false, true] {
        for has_sibling in [false, true] {
            let sibling = if has_sibling { "        after()\n" } else { "" };
            let exit = if generator { "yield 1" } else { "return 1" };
            let source = format!(
                "def f(manager,flag):\n    try:\n        try:\n            work()\n        finally:\n            try:\n                if flag:\n                    pass\n            finally:\n                pass\n{sibling}    except Exception:\n        recover()\n    {exit}\n"
            );
            for optimization_level in LEVELS {
                let [native, actual] = compile_both(
                    &source,
                    "legacy_normal_finalizer.py",
                    PythonVersion::Py312,
                    optimization_level,
                );
                assert_eq!(
                    actual, native,
                    "{optimization_level:?}, {generator}, {has_sibling}"
                );
                assert_runs(
                    probe(0)
                        .args(["-I", "-S", "-B", "-c", TRACE])
                        .args([
                            if generator { "1" } else { "0" },
                            if has_sibling { "1" } else { "0" },
                        ])
                        .stdout(Stdio::piped()),
                    [&native, &actual],
                );
            }
        }
    }
}

#[test]
fn legacy_generator_terminal_finally_keeps_enclosing_owner() {
    const TRACE: &str = r"
import dis, marshal, sys

events = []
class Manager:
    def __enter__(self):
        events.append('enter')
        return self
    def __exit__(self, typ, value, traceback):
        events.append(typ.__name__ if typ else None)
        return True
def recover():
    events.append('recover')
scope = {'recover': recover}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
line = int(sys.argv[1])
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname in ('NOP', 'JUMP_FORWARD') and i.positions.lineno == line]
if not targets:
    raise RuntimeError('missing finalizer exit')
target = min(targets)
monitor = sys.monitoring
monitor.use_tool_id(5, 'legacy-finally-owner')
for fault in (ValueError, StopIteration):
    for flag in (False, True):
        events.clear()
        hits = []
        def inject(code, offset):
            if offset == target:
                hits.append(offset)
                raise fault('injected')
        monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
        monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
        try:
            result = next(f(Manager(), flag))
        finally:
            monitor.set_local_events(5, f.__code__, 0)
            monitor.register_callback(5, monitor.events.INSTRUCTION, None)
        if hits != [target] or events != ['enter', 'AssertionError', 'recover'] or result != 1:
            raise RuntimeError((hits, events, result))
monitor.free_tool_id(5)
";
    for nested in [false, true] {
        for generator in [false, true] {
            let finalbody = if nested {
                "            try:\n                if flag:\n                    pass\n            finally:\n                pass\n"
            } else {
                "            pass\n"
            };
            let exit = if generator { "yield 1" } else { "return 1" };
            for (body, trace) in [
                (
                    "with manager:\n                pass\n                assert False",
                    true,
                ),
                (
                    "with manager as value:\n                pass\n                assert False",
                    false,
                ),
                (
                    "with manager, manager:\n                pass\n                assert False",
                    false,
                ),
                ("with manager:\n                assert 0", false),
            ] {
                let source = format!(
                    "def f(manager,flag):\n    try:\n        try:\n            {body}\n        finally:\n{finalbody}    except Exception:\n        recover()\n    {exit}\n"
                );
                let marker = source
                    .lines()
                    .enumerate()
                    .filter_map(|(index, line)| (line.trim() == "pass").then_some(index + 1))
                    .last()
                    .unwrap();
                for (index, version) in [PythonVersion::Py312, PythonVersion::Py313]
                    .into_iter()
                    .enumerate()
                {
                    for level in LEVELS {
                        let [native, actual] =
                            compile_both(&source, "suppressed_assert.py", version, level);
                        assert_eq!(
                            actual, native,
                            "{version}, {level:?}, {nested}, {generator}"
                        );
                        if !trace
                            || version != PythonVersion::Py312
                            || level != OptimizationLevel::Zero
                            || !generator
                        {
                            continue;
                        }
                        assert_runs(
                            probe(index)
                                .args(["-I", "-S", "-B", "-c", TRACE, &marker.to_string()])
                                .stdout(Stdio::piped()),
                            [&native, &actual],
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn terminal_finally_pass_is_outside_its_cleanup_handler() {
    const SOURCE: &str = "def f(flag):\n    try:\n        def inner():\n            return flag\n        result = flag\n        pass\n    finally:\n        cleanup()\n";
    const TRACE: &str = r"
import dis, json, marshal, sys
events = []
class Fault(Exception): pass
def cleanup(): events.append('cleanup')
scope = {'cleanup': cleanup}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
for opcode, line, expected in [('MAKE_FUNCTION', 3, ['cleanup', 'escaped']),
                               ('LOAD_DEREF', 5, ['cleanup', 'escaped']),
                               ('NOP', 6, ['escaped'])]:
    targets = [i.offset for i in dis.get_instructions(f)
               if i.opname == opcode and i.positions.lineno == line]
    assert len(targets) == 1, (opcode, line, targets)
    hits = []
    def trace(frame, event, arg):
        if frame.f_code is f.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == targets[0]:
                hits.append(frame.f_lasti)
                raise Fault()
        return trace
    events.clear()
    sys.settrace(trace)
    try:
        f(True)
    except Fault:
        events.append('escaped')
    finally:
        sys.settrace(None)
    assert len(hits) == 1, (opcode, hits)
    assert events == expected, (opcode, events, expected)
print(json.dumps('ok'))
";
    for (index, (version, _, _)) in VERSIONS.iter().copied().enumerate().skip(2) {
        for optimization_level in LEVELS {
            let [native, actual] = compile_both(
                SOURCE,
                "terminal_finally_pass.py",
                version,
                optimization_level,
            );
            let output = assert_outputs_match(
                probe(index).args(["-I", "-S", "-c", TRACE]),
                [&native, &actual],
            );
            assert_eq!(output, b"\"ok\"\n");
        }
    }
}

#[test]
fn named_handler_setups_preserve_optimized_backedge_locations() {
    let source = "def f(items, first, second):\n    for item in items:\n        try:\n            work(item)\n        except ValueError as error:\n            recover(error)\n        if first:\n            assert item\n        if second:\n            assert item\n    return finish()\n";
    for source in [
        source.to_owned(),
        source.replace(
            "            recover(error)",
            "            try:\n                recover(error)\n            except TypeError as other:\n                consume(other)",
        ),
        source.replace(
            "        if first:",
            "        except TypeError as other:\n            recover(other)\n        if first:",
        ),
        source.replace(
            "        if first:",
            "        try:\n            value = convert(item)\n        except TypeError as other:\n            value = str(other)\n        if first:",
        ),
        source.replace("    return finish()", "        pass\n    return finish()"),
        source.replace(
            "    return finish()",
            "        consume(item)\n    return finish()",
        ),
    ] {
        assert_matches_cpython(&source, "named_handler_backedge.py", &LEVELS);
    }
}

#[test]
fn preceding_exception_setup_does_not_own_later_backedge() {
    const SOURCE: &str = "def f(xs, work, handler):\n    try:\n        work()\n    except ValueError:\n        handler()\n    for x in xs:\n        if x:\n            assert False\n        if x:\n            assert False\n";
    const TRACE: &str = r"
import dis, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
backedges = [i.offset for i in dis.get_instructions(f)
             if i.opname == 'JUMP_BACKWARD' and i.positions.lineno is None]
if len(backedges) != 1:
    raise RuntimeError(('backedges', backedges))
target = backedges[0]
if any(e.start <= target < e.end for e in dis.Bytecode(f).exception_entries):
    raise RuntimeError('earlier handler owns later backedge')
class Fault(Exception): pass
monitor = sys.monitoring
monitor.use_tool_id(5, 'backedge-owner')
for fail in (False, True):
    events, hits = [], []
    def work():
        events.append('work')
        if fail:
            raise ValueError()
    def handler():
        events.append('handler')
    def trace(code, offset):
        if offset == target:
            hits.append(offset)
            raise Fault()
    monitor.register_callback(5, monitor.events.INSTRUCTION, trace)
    monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
    try:
        try:
            f([True, True], work, handler)
            raise RuntimeError('backedge was not reached')
        except Fault:
            pass
    finally:
        monitor.set_local_events(5, f.__code__, 0)
        monitor.register_callback(5, monitor.events.INSTRUCTION, None)
    expected = ['work', 'handler'] if fail else ['work']
    if events != expected or hits != [target]:
        raise RuntimeError((events, hits, expected))
monitor.free_tool_id(5)
";
    for level in [OptimizationLevel::One, OptimizationLevel::Two] {
        let [native, actual] = compile_both(
            SOURCE,
            "preceding_exception_trace.py",
            PythonVersion::Py312,
            level,
        );
        assert_eq!(actual, native);
        assert_runs(
            probe(0).args(["-I", "-S", "-B", "-c", TRACE]),
            [&native, &actual],
        );
    }
}

#[test]
fn generator_backedge_after_exception_setup_has_no_line() {
    const SOURCE: &str = "def f(xs, work, recover):\n try:\n  work()\n except ValueError:\n  recover()\n for x in xs:\n  if x:\n   assert False\n  if x:\n   assert False\n yield 1\n";
    const TRACE: &str = r"
import dis, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'JUMP_BACKWARD' and i.positions.lineno is None]
if len(targets) != 1:
    raise RuntimeError(('backedges', targets))
target = targets[0]
class Fault(Exception): pass
monitor = sys.monitoring
monitor.use_tool_id(5, 'generator-backedge')
for fail in (False, True):
    for exception in (Fault, StopIteration):
        events, hits, traces = [], [], []
        def work():
            events.append('work')
            if fail:
                raise ValueError()
        def recover():
            events.append('recover')
        def inject(code, offset):
            if code is f.__code__ and offset == target:
                hits.append(offset)
                raise exception('injected')
        def tracer(frame, event, arg):
            if frame.f_code is f.__code__ and event in ('exception', 'return'):
                traces.append((event, frame.f_lineno))
            return tracer
        monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
        monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
        sys.settrace(tracer)
        try:
            try:
                list(f([True, True], work, recover))
                raise RuntimeError('backedge was not reached')
            except Fault:
                if exception is not Fault:
                    raise
            except RuntimeError as e:
                if exception is not StopIteration or not isinstance(e.__cause__, StopIteration):
                    raise
        finally:
            sys.settrace(None)
            monitor.set_local_events(5, f.__code__, 0)
            monitor.register_callback(5, monitor.events.INSTRUCTION, None)
        expected = ['work', 'recover'] if fail else ['work']
        if events != expected or hits != [target] or traces[-2:] != [('exception', None), ('return', None)]:
            raise RuntimeError((events, hits, traces, expected))
monitor.free_tool_id(5)
";
    for level in [OptimizationLevel::One, OptimizationLevel::Two] {
        let [native, actual] =
            compile_both(SOURCE, "generator_backedge.py", PythonVersion::Py312, level);
        assert_eq!(actual, native);
        assert_runs(
            probe(0).args(["-I", "-S", "-B", "-c", TRACE]),
            [&native, &actual],
        );
    }
}

#[test]
fn folded_match_guard_preserves_original_nop() {
    let source = r#"class Example:
    @property
    def case(self):
        match self.state.data.kind:
            case Kind.FIRST if True:
                assert isinstance(self.state.data, First)
            case Kind.SECOND:
                work(self.state.data)
            case Kind.THIRD:
                work(self.state.data)
            case _:
                raise AssertionError(f"unknown: {self.state.data.kind}")
        return self.state.data
"#;
    for source in [
        source.to_owned(),
        source.replace("return self.state.data", "return (self, value)"),
        source.replace("return self.state.data", "return self.state.data.payload"),
        source.replace(
            "return self.state.data",
            "return (\n            self.state.data\n        )",
        ),
        source.replace("if True:", "if (\n                True\n            ):"),
        source.replace("if True:", "if valid(self):"),
        source.replace("assert isinstance(self.state.data, First)", "pass"),
        source.replace("work(self.state.data)", "assert self.state.data"),
    ] {
        assert_matches_cpython(&source, "folded_match_guard.py", &LEVELS);
    }
}

#[test]
fn erased_match_cases_inherit_copied_return_locations() {
    let source = r#"class Example:
    @property
    def case(self):
        match self.state.data.kind:
            case Kind.FIRST:
                assert isinstance(self.state.data, First)
            case Kind.SECOND:
                assert isinstance(self.state.data, Second)
            case Kind.THIRD:
                assert isinstance(self.state.data, Third)
            case _:
                raise AssertionError(f"unknown: {self.state.data.kind}")
        return self.state.data
"#;
    for source in [
        source.to_owned(),
        source.replace("return self.state.data", "return self.state"),
        source.replace("return self.state.data", "return self.state.data.payload"),
        source.replace("case Kind.FIRST:", "case None:"),
        source.replace("case Kind.FIRST:", "case [value]:"),
        source.replace("case Kind.FIRST:", "case Kind.FIRST if valid(self):"),
        source.replace("assert isinstance(self.state.data, First)", "pass"),
        source.replace(
            "        return self.state.data",
            "        work(self)\n        return self.state.data",
        ),
    ] {
        assert_matches_cpython(&source, "copied_match_return.py", &LEVELS);
    }
}

#[test]
fn coroutine_erased_else_preserves_nested_backedge_donor() {
    let source = r#"async def consume(records):
    done = False
    for record in records:
        if not record:
            continue
        if not done:
            record = record[2:]
            try:
                event = parse(record)
            except ValueError:
                continue
            if event.kind == "piece":
                accept(record)
            elif event.kind == "end":
                result = finish(record)
                done = True
                assert result.valid
            else:
                assert False
        else:
            assert record == "end"
"#;
    for source in [
        source.to_owned(),
        source.replace("async def", "def"),
        source.replace("                assert result.valid\n", ""),
        source.replace(
            "elif event.kind == \"end\":",
            "elif event.kind == \"end\" or ready(record):",
        ),
        source.replace(
            "elif event.kind == \"end\":",
            "elif (\n                event.kind\n                == \"end\"\n            ):",
        ),
        source.replace(
            "                assert False",
            "                reject(record)",
        ),
    ] {
        assert_matches_cpython(&source, "coroutine_erased_else_donor.py", &LEVELS);
    }
}

#[test]
fn elided_inner_else_in_for_else_matches_cpython() {
    for (prefix, finalbody) in [
        ("", "        finish()\n"),
        (
            "            if c:\n                break\n",
            "        finish()\n",
        ),
        (
            "            if c:\n                continue\n",
            "        finish()\n",
        ),
        (
            "",
            "        try:\n            finish()\n        finally:\n            cleanup()\n",
        ),
    ] {
        let source = format!(
            "def f(xs, a, b, c):\n    z = 0\n    for x in xs:\n        if a:\n{prefix}            if b:\n                z = x\n            else:\n                assert b\n        else:\n            work(x)\n    else:\n{finalbody}"
        );
        assert_matches_cpython_versions(
            &source,
            "elided_inner_else_for_else.py",
            &LEVELS,
            &VERSIONS.map(|(version, _, _)| version),
        );
    }
}

#[test]
fn elided_inner_else_for_else_preserves_backedge_trace() {
    const SOURCE: &str = "def f(xs, a, b, c):\n    z = 0\n    for x in xs:\n        if a:\n            if c:\n                break\n            if b:\n                z = x\n            else:\n                assert b\n        else:\n            work(x)\n    else:\n        finish()\n";
    const TRACE: &str = r"
import dis, marshal, sys
scope = {}
events = []
def finish(): events.append('finish')
def work(x): events.append(('work', x))
scope.update(finish=finish, work=work)
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname.startswith('JUMP_BACKWARD') and i.positions.lineno == 7]
if len(targets) != 1:
    raise RuntimeError(('backedge', targets))
target = targets[0]
if any(e.start <= target < e.end for e in dis.Bytecode(f).exception_entries):
    raise RuntimeError('backedge unexpectedly has an exception owner')
class Fault(Exception): pass
hits = []
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        frame.f_trace_opcodes = True
        if event == 'opcode' and frame.f_lasti == target:
            hits.append(target)
            raise Fault()
    return trace
sys.settrace(trace)
try:
    try:
        f([1, 2], True, False, False)
        raise RuntimeError('injection missed')
    except Fault:
        pass
finally:
    sys.settrace(None)
if hits != [target] or events:
    raise RuntimeError((hits, events))
f([1, 2], True, False, True)
if events:
    raise RuntimeError(('break ran loop else', events))
f([1, 2], True, False, False)
if events != ['finish']:
    raise RuntimeError(('normal loop else', events))
";
    for python_version in [PythonVersion::Py314, PythonVersion::Py315] {
        let index = VERSIONS
            .iter()
            .position(|(version, _, _)| *version == python_version)
            .unwrap();
        for optimization_level in [OptimizationLevel::One, OptimizationLevel::Two] {
            let [native, actual] = compile_both(
                SOURCE,
                "elided_inner_else_for_else.py",
                python_version,
                optimization_level,
            );
            assert_eq!(actual, native);
            assert_runs(
                probe(index).args(["-I", "-S", "-c", TRACE]),
                [&native, &actual],
            );
        }
    }
}

#[test]
fn elided_assertions_preserve_terminal_branch_return_locations() {
    for (index, source) in [
        "def f(first, second, obj):\n    if first:\n        if second:\n            value = obj.first\n        else:\n            value = obj.second\n        assert value\n",
        "def f(first, second):\n    if first:\n        while second:\n            second = step()\n        assert second\n",
        "def f(first, second, obj):\n    if first:\n        work()\n    else:\n        if second:\n            value = obj.first\n        else:\n            value = obj.second\n        assert value\n",
        "def f(first, second):\n    if first:\n        work()\n    else:\n        while second:\n            second = step()\n        assert second\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("terminal_branch_return_{index}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn terminating_loop_branch_before_elided_assert_matches_cpython() {
    for terminal in [
        "raise ValueError()",
        "raise ValueError() from flag",
        "return item",
        "return",
        "return item or flag",
        "return item if flag else items",
    ] {
        for otherwise in ["", "        else:\n            assert items\n"] {
            let source = format!(
                "def f(items, flag):
    for item in items:
        if flag:
            if item is None:
                {terminal}
            assert item
{otherwise}"
            );
            assert_matches_cpython(&source, "terminating_elided_loop_branch.py", &LEVELS);
        }
    }
}

#[test]
fn terminal_if_before_elided_assert_keeps_manager_exception_owner_312() {
    const SOURCE: &str = "def f(x):\n if x:\n  with context():\n   call()\n else:\n  if x:\n   assert x\n  assert x\n";
    const TRACE: &str = r"
import dis, json, marshal, sys
events = []
class Manager:
    def __enter__(self):
        events.append('enter')
    def __exit__(self, kind, value, traceback):
        events.append('exit:' + kind.__name__)
        return False
scope = {'context': Manager, 'call': lambda: events.append('body')}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
target = next(i.offset for i in dis.get_instructions(f)
              if i.opname == 'LOAD_GLOBAL' and i.argval == 'call')
fired = [False]
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        if event == 'call':
            frame.f_trace_opcodes = True
        elif event == 'opcode' and frame.f_lasti == target:
            fired[0] = True
            raise ValueError('injected')
    return trace
sys.settrace(trace)
try:
    f(False)
except AssertionError:
    pass
sys.settrace(None)
events.clear()
sys.settrace(trace)
try:
    f(True)
except ValueError:
    events.append('escaped')
finally:
    sys.settrace(None)
assert fired[0]
print(json.dumps(events))
";
    for optimization_level in LEVELS {
        let [native, actual] = compile_both(
            SOURCE,
            "terminal_elided_assert.py",
            PythonVersion::Py312,
            optimization_level,
        );
        let output =
            assert_outputs_match(probe(0).args(["-I", "-S", "-c", TRACE]), [&native, &actual]);
        assert_eq!(output, b"[\"enter\", \"exit:ValueError\", \"escaped\"]\n");
    }
}

#[test]
fn elided_loop_arms_inside_managers_match_cpython() {
    for test in [
        "item",
        "item == 1",
        "item is None",
        "item and flag",
        "item or flag",
    ] {
        for prefix in [
            "",
            "            first = setup()\n            second = setup()\n",
        ] {
            let source = format!(
                "def f(manager, items, flag):
    with manager:
        with manager:
{prefix}            for item in items:
                if {test}:
                    assert flag
                    assert item
                else:
                    assert flag
                    assert item
"
            );
            assert_matches_cpython(&source, "elided_manager_loop.py", &LEVELS);
        }
    }
}

#[test]
fn elided_manager_loop_arms_preserve_conditional_successor_nops() {
    for gap in ["", "            assert a\n"] {
        for successor in [
            "if b:\n                assert b\n            else:\n                assert a",
            "if b:\n                break",
            "if b and c:\n                work()",
        ] {
            let source = format!(
                "def f(cm, items, a, b, c):\n    with cm:\n        for item in items:\n            if a:\n                assert a\n            else:\n                assert b\n{gap}            {successor}\n"
            );
            assert_matches_cpython(&source, "elided_manager_conditional_successor.py", &LEVELS);
        }
    }
}

#[test]
fn coroutine_elided_loop_arms_inside_managers_match_cpython() {
    for manager in ["with cm", "async with cm"] {
        for condition in ["item == 1", "first or second"] {
            let source = format!(
                "async def f(cm, items, first, second):\n    {manager}:\n        for item in items:\n            if {condition}:\n                assert item\n            else:\n                assert items\n"
            );
            assert_matches_cpython(&source, "coroutine_elided_manager_loop.py", &LEVELS);
        }
    }
}

#[test]
fn optimized_elided_join_preserves_exception_owners() {
    const SOURCE: &str = r"
def sync(flag):
    with manager('outer'):
        with manager('inner'):
            if flag:
                assert flag
            else:
                assert not flag
            return 1

def terminal(flag):
    with manager('terminal'):
        if flag:
            assert flag
        else:
            assert not flag

async def async_terminal(flag):
    async with manager('async'):
        if flag:
            assert flag
        else:
            assert not flag

def same_line(flag):
    with manager('same-line'):
        if flag: assert flag
        else: assert not flag

def multiple(flag):
    with manager('left'), manager('right'):
        if flag:
            assert flag
        else:
            assert not flag

def gen(flag):
    with manager('generator'):
        if flag:
            assert flag
        else:
            assert not flag
        yield 1
";
    const TRACE_JOIN: &str = r"
import asyncio, dis, marshal, sys

events = []
class Manager:
    def __init__(self, name):
        self.name = name
    def __enter__(self):
        events.append((self.name, 'enter'))
    def __exit__(self, typ, value, tb):
        events.append((self.name, 'exit', typ.__name__ if typ else None))
        return False
    async def __aenter__(self):
        events.append((self.name, 'enter'))
    async def __aexit__(self, typ, value, tb):
        events.append((self.name, 'exit', typ.__name__ if typ else None))
        return False

scope = {'manager': Manager}
exec(marshal.loads(sys.stdin.buffer.read()), scope)

def trace_join(fn, injected):
    instructions = list(dis.get_instructions(fn))
    offsets = [b.offset for a, b in zip(instructions, instructions[1:])
               if a.opname == 'NOT_TAKEN' and b.opname == 'NOP']
    assert len(offsets) == 1, offsets
    def trace(frame, event, arg):
        if frame.f_code is not fn.__code__:
            return None
        if event == 'call':
            frame.f_trace_opcodes = True
        elif event == 'opcode' and frame.f_lasti == offsets[0] and injected:
            events.append('hit')
            raise injected('injected')
        return trace
    return trace

for name, injected, expected in [
    ('sync', ValueError, [('outer', 'enter'), ('inner', 'enter'), 'hit',
                          ('inner', 'exit', 'ValueError'), ('outer', 'exit', 'ValueError'),
                          ('caught', 'ValueError', None)]),
    ('terminal', ValueError, [('terminal', 'enter'), 'hit',
                              ('terminal', 'exit', 'ValueError'), ('caught', 'ValueError', None)]),
    ('async_terminal', ValueError, [('async', 'enter'), 'hit',
                                    ('async', 'exit', 'ValueError'), ('caught', 'ValueError', None)]),
    ('same_line', ValueError, [('same-line', 'enter'), 'hit',
                               ('same-line', 'exit', 'ValueError'), ('caught', 'ValueError', None)]),
    ('multiple', ValueError, [('left', 'enter'), ('right', 'enter'), 'hit',
                              ('right', 'exit', 'ValueError'), ('left', 'exit', 'ValueError'),
                              ('caught', 'ValueError', None)]),
    ('gen', StopIteration, [('generator', 'enter'), 'hit',
                            ('generator', 'exit', 'StopIteration'),
                            ('caught', 'RuntimeError', 'StopIteration')]),
]:
    fn = scope[name]
    def invoke():
        result = fn(True)
        if name == 'gen':
            return next(result)
        if name == 'async_terminal':
            return asyncio.run(result)
        return result
    # Initialize opcode tracing for this code object before injecting an exception.
    sys.settrace(trace_join(fn, None))
    try:
        invoke()
    finally:
        sys.settrace(None)
    events.clear()
    sys.settrace(trace_join(fn, injected))
    try:
        invoke()
    except BaseException as error:
        events.append(('caught', type(error).__name__,
                       type(error.__cause__).__name__ if error.__cause__ else None))
    finally:
        sys.settrace(None)
    assert events == expected, (events, expected)
";
    for (version, level) in [
        (PythonVersion::Py314, OptimizationLevel::One),
        (PythonVersion::Py314, OptimizationLevel::Two),
        (PythonVersion::Py315, OptimizationLevel::One),
        (PythonVersion::Py315, OptimizationLevel::Two),
    ] {
        let [native, actual] = compile_both(SOURCE, "optimized_join.py", version, level);
        assert_eq!(actual, native);
        let index = VERSIONS
            .iter()
            .position(|(target, _, _)| *target == version)
            .unwrap();
        assert_runs(
            probe(index)
                .args(["-I", "-S", "-X", "utf8", "-c", TRACE_JOIN])
                .stdout(Stdio::piped()),
            [&native, &actual],
        );
    }
}

#[test]
fn protected_optimized_conditional_keeps_nested_exception_owners() {
    const SOURCE: &str = r"events = []
class Fault(Exception): pass
class M:
 def __enter__(self): events.append('enter'); return self
 def __exit__(self, typ, value, tb): events.append('exit:' + (typ.__name__ if typ else 'none')); return False
def step(): events.append('step')
def other(): events.append('other')
def f(a):
 try:
  if a:
   with M():
    try:
     step()
    except KeyError:
     other()
  else:
   other()
  assert a
 except Fault:
  events.append('handler')
";
    const FOLDED_SOURCE: &str = r"events = []
class Fault(Exception): pass
class M:
 def __enter__(self): events.append('enter'); return self
 def __exit__(self, typ, value, tb): events.append('exit:' + (typ.__name__ if typ else 'none')); return False
def step(): events.append('step')
def other(): events.append('other')
def f(a):
 try:
  if True:
   if a:
    with M():
     try:
      step()
     except KeyError:
      other()
   else:
    other()
  else:
   other()
  assert a
 except Fault:
  events.append('handler')
";
    const TRACE: &str = r"
import dis, marshal, sys
scope = {}
exec(marshal.load(sys.stdin.buffer), scope)
f, fault, events = scope['f'], scope['Fault'], scope['events']
cases = [('POP_JUMP', int(sys.argv[1]), False, ['handler']),
         ('CALL', int(sys.argv[2]), True, ['enter', 'exit:Fault', 'handler'])]
for prefix, line, argument, expected in cases:
    matches = [instruction.offset for instruction in dis.get_instructions(f)
               if instruction.opname.startswith(prefix) and instruction.positions.lineno == line]
    if len(matches) != 1:
        raise RuntimeError((prefix, line, matches))
    target, hits = matches[0], []
    events.clear()
    def trace(frame, event, arg):
        if frame.f_code is f.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == target:
                hits.append(target)
                raise fault('injected')
        return trace
    try:
        sys.settrace(trace)
        result = f(argument)
    finally:
        sys.settrace(None)
    if hits != [target] or result is not None or events != expected:
        raise RuntimeError((prefix, hits, result, events, expected))
";
    for source in [SOURCE, FOLDED_SOURCE] {
        let lines = ["if a:", "step()"].map(|needle| {
            (source
                .lines()
                .position(|line| line.trim() == needle)
                .unwrap()
                + 1)
            .to_string()
        });
        for version in [PythonVersion::Py314, PythonVersion::Py315] {
            for level in [OptimizationLevel::One, OptimizationLevel::Two] {
                let [native, actual] = compile_both(source, "protected_owner.py", version, level);
                assert_eq!(actual, native, "{version}, {level:?}");
                let index = VERSIONS
                    .iter()
                    .position(|(candidate, _, _)| *candidate == version)
                    .unwrap();
                assert_runs(
                    probe(index)
                        .args(["-I", "-S", "-B", "-c", TRACE])
                        .args(&lines)
                        .stdout(Stdio::piped()),
                    [&native, &actual],
                );
            }
        }
    }
}

#[test]
fn folded_protected_nested_terminal_joins_match_cpython() {
    for (index, source) in [
        "def f(a, b):\n try:\n  if True:\n   if b:\n    work()\n  else:\n   other()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, b):\n try:\n  if True:\n   if b:\n    work()\n   else:\n    other()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, b):\n try:\n  if True:\n   while b:\n    break\n  else:\n   other()\n  assert a\n except ValueError:\n  work()\n",
    ].into_iter().enumerate() {
        assert_matches_cpython_versions(
            source,
            &format!("sub/folded_protected_nested_{index}.py"),
            &LEVELS[1..],
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn protected_elided_assertion_preserves_terminal_control_flow() {
    for (index, source) in [
        "def f(a, ys):\n try:\n  if not a:\n   x = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
        "def f(obj, ys):\n try:\n  if obj.flag:\n   x = [i for i in ys]\n  assert obj\n except ValueError:\n  work()\n",
        "def f(a, b, ys):\n try:\n  if a and b:\n   x = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, ys):\n try:\n  if bool(a):\n   x = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, obj, ys):\n try:\n  if a:\n   obj.value = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, obj, ys):\n try:\n  if a:\n   obj[0] = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, ys):\n try:\n  if a:\n   x, = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, ys):\n try:\n  if a:\n   x = [i for i in ys]\n  else:\n   x = []\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if a:\n   work()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, b):\n try:\n  if a:\n   work()\n  elif b:\n   other()\n  else:\n   third()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, b):\n try:\n  if a == b:\n   work()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if a:\n   return 1\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if a:\n   pass\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, b):\n try:\n  if a:\n   if b:\n    work()\n  assert a\n  assert b\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if True:\n   pass\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if False:\n   work()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, ys):\n try:\n  if __debug__:\n   x = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if a:\n   with manager():\n    work()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if a:\n   try:\n    work()\n   except TypeError:\n    other()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if a:\n   try:\n    work()\n   finally:\n    other()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, ys):\n try:\n  if a:\n   for y in ys:\n    work()\n  assert a\n except ValueError:\n  work()\n",
    ].into_iter().enumerate() {
        assert_matches_cpython_versions(
            source,
            &format!("sub/protected_terminal_elided_{index}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn folded_protected_branch_before_elided_assertion_keeps_exit_location() {
    let versions = VERSIONS.map(|(version, _, _)| version);
    for (index, source) in [
        "def f(a):\n try:\n  if True:\n   pass\n  else:\n   other()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, b):\n try:\n  if 1:\n   pass\n  else:\n   pass\n  assert a\n  assert b\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if True:\n   pass\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if False:\n   work()\n  else:\n   pass\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if __debug__:\n   work()\n  else:\n   pass\n  assert a\n except ValueError:\n  work()\n",
    ].into_iter().enumerate() {
        assert_matches_cpython_versions(
            source,
            &format!("sub/folded_protected_exit_{index}.py"),
            &LEVELS,
            &versions,
        );
    }
    for (index, source) in [
        "def f(a):\n try:\n  try:\n   if True:\n    pass\n   else:\n    other()\n   assert a\n  except ValueError:\n   work()\n except TypeError:\n  other()\n",
        "def f(a, obj):\n with obj:\n  try:\n   if True:\n    pass\n   else:\n    other()\n   assert a\n  except ValueError:\n   work()\n",
        "def f(a, xs):\n for x in xs:\n  try:\n   if True:\n    pass\n   else:\n    other()\n   assert a\n  except ValueError:\n   work()\n",
        "def f(a):\n yield 1\n try:\n  if True:\n   pass\n  else:\n   other()\n  assert a\n except ValueError:\n  work()\n",
        "async def f(a):\n try:\n  if True:\n   pass\n  else:\n   other()\n  assert a\n except ValueError:\n  work()\n",
    ].into_iter().enumerate() {
        assert_matches_cpython_versions(
            source,
            &format!("sub/folded_protected_context_{index}.py"),
            &LEVELS[1..],
            &versions,
        );
    }
}

#[test]
fn folded_try_pass_after_branch_keeps_existing_exception_owner() {
    const TRACE: &str = r"
import dis, marshal, sys, types
sys.modules['synthetic_module'] = types.ModuleType('synthetic_module')
events = []
class Manager:
    def __enter__(self):
        events.append('enter')
        return self
    def __exit__(self, kind, value, traceback):
        events.append('exit:' + (kind.__name__ if kind else 'none'))
        return False
scope = {'recover': lambda: events.append('recover'), 'other': lambda: events.append('other')}
exec(marshal.load(sys.stdin.buffer), scope)
f = scope['outer']()
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOP' and i.positions.lineno == int(sys.argv[1])]
if len(targets) != 1:
    raise RuntimeError(('expected one Pass NOP', targets))
target, hits = targets[0], []
def inject(code, offset):
    if offset == target:
        hits.append(offset)
        raise ValueError('injected')
monitor = sys.monitoring
monitor.use_tool_id(5, 'folded-pass-owner')
monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
try:
    values = list(f(sys.argv[2] == 'true', Manager(), []))
finally:
    monitor.set_local_events(5, f.__code__, 0)
    monitor.register_callback(5, monitor.events.INSTRUCTION, None)
    monitor.free_tool_id(5)
if hits != [target] or values != [1] or events != ['enter', 'recover', 'exit:none']:
    raise RuntimeError((hits, values, events))
";
    for (body, target_line, argument) in [
        ("if a:\n      pass", "9", "true"),
        (
            "if a:\n      pass\n     else:\n      pass\n     pass",
            "11",
            "false",
        ),
        ("if a: pass\n     pass; pass", "8", "true"),
    ] {
        let source = format!(
            "from synthetic_module import *\ndef outer():\n marker = 1\n def f(a, manager, xs):\n  with manager:\n   try:\n    if True:\n     {body}\n    else:\n     other()\n    assert a\n   except ValueError:\n    recover()\n  for x in xs:\n   if x:\n    assert False\n   if x:\n    assert False\n  yield 1\n return f\n"
        );
        for version in [PythonVersion::Py314, PythonVersion::Py315] {
            for level in [OptimizationLevel::One, OptimizationLevel::Two] {
                let [native, actual] =
                    compile_both(&source, "folded_pass_owner.py", version, level);
                let index = VERSIONS
                    .iter()
                    .position(|(candidate, _, _)| *candidate == version)
                    .unwrap();
                assert_runs(
                    probe(index)
                        .args(["-I", "-S", "-B", "-c", TRACE, target_line, argument])
                        .stdout(Stdio::piped()),
                    [&native, &actual],
                );
            }
        }
    }
}

#[test]
fn optimized_legacy_await_assertions_preserve_coroutine_symbols() {
    let sources = [
        "def f():\n    assert await g()\n",
        "def f():\n    assert condition, (await g())\n",
        "def f():\n    assert 1 and (await g())\n",
        "def f():\n    assert [await g() for x in xs]\n",
        "def f():\n    assert [x for x in await g()]\n",
        "def f():\n    assert [x async for x in xs]\n",
        "def f():\n    assert {x async for x in xs}\n",
        "def f():\n    assert {x: x async for x in xs}\n",
        "def f():\n    assert (x for x in await g())\n",
        "def f():\n    assert (await g() for x in xs)\n",
        "def f():\n    assert (x async for x in xs)\n",
        "def f():\n    assert [x for x in (x async for x in xs)]\n",
        "def f():\n    assert (lambda: (await g()))\n",
        "def f():\n    assert (lambda x=(await g()): x)\n",
        "def f():\n    assert [(lambda: (await g())) for x in xs]\n",
        "def f():\n    assert (lambda x=[y async for y in ys]: x)\n",
        "def f():\n    assert (yield 1), (await g())\n",
        "def f():\n    assert (yield from (await g()))\n",
        "def f():\n    assert (await (yield from g()))\n",
        "def f():\n    assert (x async for x in await g())\n",
        "def f():\n    assert [await x async for x in xs]\n",
        "def f():\n    assert (lambda x=(await g()): (yield 1))\n",
        "def f():\n    if condition:\n        assert await g()\n    return 1\n",
        "def f():\n    if False:\n        assert await g()\n    return 1\n",
        "def outer():\n    def inner():\n        assert await g()\n    return inner\n",
        "class C:\n    def method(self):\n        assert await g()\n",
        "def f():\n    assert condition\n",
        "def f():\n    assert (lambda: (await g()))\n    yield 1\n",
        "def f():\n    assert await g()\n    def inner(): yield 1\n    return 3\n",
        "def f():\n    assert await g()\n    yield 1\n    return\n",
        "def f():\n    assert await g()\n    yield from xs\n",
        "def ordinary():\n    assert condition\n    yield 1\n    return 3\ndef special():\n    assert await g()\n",
        "def ordinary():\n    assert condition\n    yield 1\n    return 3\n# async await\n",
    ];
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        for level in [OptimizationLevel::One, OptimizationLevel::Two] {
            for source in sources {
                let filename = "legacy_assert_await.py";
                let expected = oracle(source.as_bytes(), filename, version, level);
                assert!(
                    expected.status.success(),
                    "{version} {level:?}: {source}: {expected:?}"
                );
                let actual = compile_with_options(
                    source,
                    filename,
                    &CompileOptions {
                        python_version: version,
                        optimization_level: level,
                        ..CompileOptions::default()
                    },
                )
                .unwrap()
                .marshal();
                assert_eq!(actual, expected.stdout, "{version} {level:?}: {source}");
            }
        }
    }
}

#[test]
fn optimized_legacy_await_and_yield_reject_value_returns() {
    for source in [
        "def f():\n    assert await g()\n    yield 1\n    return 3\n",
        "def f():\n    assert await g()\n    return (yield 1)\n",
        "def f():\n    assert await g()\n    yield 1\n    if False:\n        return 3\n",
        "def f():\n    assert await g()\n    def inner(x=(yield 1)): pass\n    return 3\n",
    ] {
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            for level in [OptimizationLevel::One, OptimizationLevel::Two] {
                let expected = oracle(source.as_bytes(), "legacy_assert_await.py", version, level);
                assert!(!expected.status.success());
                assert!(
                    String::from_utf8_lossy(&expected.stderr)
                        .contains("'return' with value in async generator")
                );
                let error = compile_with_options(
                    source,
                    "legacy_assert_await.py",
                    &CompileOptions {
                        python_version: version,
                        optimization_level: level,
                        ..CompileOptions::default()
                    },
                )
                .unwrap_err();
                let serc::CompileError::InvalidSyntax { message, range } = error else {
                    panic!("{version} {level:?}: {error:?}");
                };
                assert_eq!(message, "`return` with value in async generator");
                assert_eq!(range.start().to_usize(), source.rfind("return ").unwrap());
                assert_eq!(range.end().to_usize(), source.trim_end().len());
            }
        }
    }
}

#[test]
fn optimized_legacy_await_return_preserves_symbol_error_precedence() {
    for (source, messages) in [
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    global x\n    x = 2\n    global x\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    print(x)\n    global x\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    nonlocal x\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    [(yield x) for x in xs]\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    [x for x in (x := xs)]\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    [(x := 2) for x in xs]\n",
            Some((
                "assignment expression cannot rebind comprehension iteration variable 'x'",
                "assignment expression cannot rebind comprehension variable",
            )),
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    x = 2\n    global x\n    [(yield y) for y in ys]\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    [(yield y) for y in ys]\n    x = 2\n    global x\n",
            None,
        ),
    ] {
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            for level in [OptimizationLevel::One, OptimizationLevel::Two] {
                let expected = oracle(source.as_bytes(), "legacy_assert_await.py", version, level);
                let stderr = String::from_utf8_lossy(&expected.stderr);
                let expected_message = stderr
                    .lines()
                    .find_map(|line| line.strip_prefix("SyntaxError: "))
                    .unwrap_or_else(|| panic!("the pinned oracle failed unexpectedly: {stderr}"));
                let serc_message = if let Some((native, serc)) = messages {
                    // Preserve Serc's existing wording while checking both diagnostics.
                    assert_eq!(expected_message, native);
                    serc
                } else {
                    expected_message
                };
                let error = compile_with_options(
                    source,
                    "legacy_assert_await.py",
                    &CompileOptions {
                        python_version: version,
                        optimization_level: level,
                        ..CompileOptions::default()
                    },
                )
                .unwrap_err();
                assert!(
                    error.to_string().ends_with(serc_message),
                    "{version} {level:?}: {source}: expected {serc_message:?}, got {error}"
                );
            }
        }
    }
}

#[test]
fn debug_annotations_preserve_postponed_source_spelling() {
    let source =
        "value: __debug__ = 1\ndef function(arg: __debug__) -> tuple[__debug__]:\n    return arg\n";
    assert_matches_cpython(source, "optimized_annotations.py", &LEVELS);
    assert_matches_cpython(
        "assert condition\nx: int = 1\n",
        "optimized_assertion_annotation.py",
        &LEVELS,
    );
    assert_matches_cpython(
        &format!("from __future__ import annotations\n{source}"),
        "optimized_future_annotations.py",
        &LEVELS,
    );
}

#[test]
fn optimized_assertion_syntax_matches_the_target_version() {
    for source in [
        "assert (await value)\n",
        "def f():\n    assert (await g())\n",
        "assert (__debug__ := False)\n",
        "assert func(__debug__=1)\n",
        "assert func(x=1, x=2)\n",
        "assert [0 for x.__debug__ in []]\n",
        "assert (yield, 1)\n",
        "def f():\n    assert (value := 1)\n    global value\n",
    ] {
        for (python_version, _, _) in VERSIONS {
            for optimization_level in LEVELS {
                let expected = oracle(
                    source.as_bytes(),
                    "invalid_optimized_assert.py",
                    python_version,
                    optimization_level,
                );
                let options = CompileOptions {
                    python_version,
                    optimization_level,
                    ..CompileOptions::default()
                };
                let actual = compile_with_options(source, "invalid_optimized_assert.py", &options);
                if expected.status.success() {
                    assert_eq!(
                        actual.unwrap().marshal(),
                        expected.stdout,
                        "{python_version}, {optimization_level:?}, {source:?}"
                    );
                } else {
                    let stderr = String::from_utf8_lossy(&expected.stderr);
                    let expected_message = stderr
                        .lines()
                        .find_map(|line| line.strip_prefix("SyntaxError: "))
                        .unwrap_or_else(|| {
                            panic!("the pinned oracle failed unexpectedly: {stderr}")
                        });
                    let error = actual.unwrap_err();
                    assert!(
                        error.to_string().ends_with(expected_message),
                        "{python_version}, {optimization_level:?}, {source:?}: \
                         expected {expected_message:?}, got {error}"
                    );
                }
            }
        }
    }
}

#[test]
fn source_decoding_preserves_optimization_options() {
    let source = b"# coding: latin-1\r\n\"caf\xe9\"\r\nassert __debug__\r\nvalue = __debug__\r\n";
    let filename = "optimized_encoding.py";
    for (python_version, _, _) in VERSIONS {
        for optimization_level in LEVELS {
            let options = CompileOptions {
                python_version,
                optimization_level,
                ..CompileOptions::default()
            };
            let expected = oracle(source, filename, python_version, optimization_level);
            assert!(
                expected.status.success(),
                "{}",
                String::from_utf8_lossy(&expected.stderr)
            );
            for actual in [
                compile_bytes_with_options(source, filename, &options).unwrap(),
                compile_bytes_with_path_and_options(source, Path::new(filename), &options).unwrap(),
            ] {
                assert_eq!(
                    actual.marshal(),
                    expected.stdout,
                    "{python_version}, {optimization_level:?}"
                );
            }
        }
    }
}

#[test]
fn default_options_preserve_unoptimized_behavior() {
    let source = "\"module docstring\"\nassert condition\nvalue = __debug__\n";
    let default = CompileOptions::default();
    assert_eq!(default.optimization_level, OptimizationLevel::Zero);
    assert_eq!(
        compile(source, "default_optimization.py")
            .unwrap()
            .marshal(),
        compile_with_options(source, "default_optimization.py", &default)
            .unwrap()
            .marshal()
    );
}

#[test]
fn optimized_deep_expression_traversal_and_cleanup() {
    const CHILD_ENV: &str = "SERC_TEST_OPTIMIZED_DEEP_EXPRESSIONS";
    if std::env::var_os(CHILD_ENV).is_some() {
        // Exercise direct destruction at the short-expression boundary and guarded destruction
        // of longer chains, both with ample stack and with a caller that must grow its stack.
        for terms in [126, 127, 128, 129, 2_048] {
            let expression = std::iter::repeat_n("x", terms)
                .collect::<Vec<_>>()
                .join("+");
            let source = format!("result = {expression}\n");
            let expected = compile(&source, "cleanup_boundary.py").unwrap().marshal();
            for stack_size in [128 * 1024, 8 * 1024 * 1024] {
                let source = source.clone();
                let actual = std::thread::Builder::new()
                    .stack_size(stack_size)
                    .spawn(move || compile(&source, "cleanup_boundary.py").unwrap().marshal())
                    .unwrap()
                    .join()
                    .unwrap();
                assert_eq!(actual, expected);
            }
        }
        let expression = std::iter::repeat_n("value", 2_048)
            .collect::<Vec<_>>()
            .join(" + ");
        let source = format!(
            "\"module docstring\"\nvalue = 1\nassert {expression}\nresult = __debug__ + {expression}\n"
        );
        for optimization_level in [OptimizationLevel::One, OptimizationLevel::Two] {
            let options = CompileOptions {
                optimization_level,
                ..CompileOptions::default()
            };
            let module = compile_with_options(&source, "deep_optimized.py", &options).unwrap();
            assert!(!module.marshal().is_empty());
        }
        return;
    }

    // A stack-overflow regression must fail this test without aborting the entire test binary.
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "optimized_deep_expression_traversal_and_cleanup",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "optimized expression subprocess failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn optimized_empty_manager_bodies_match_cpython() {
    for (header, manager) in [
        ("def f():", "with"),
        ("async def f():", "with"),
        ("async def f():", "async with"),
    ] {
        for body in [
            "        assert predicate()\n",
            "        assert predicate()\n        assert other()\n",
            "        pass\n",
            "        global marker\n        pass\n",
            "        global marker\n        assert predicate()\n",
        ] {
            for continuation in ["", "    after = work()\n"] {
                let source = format!(
                    "{header}\n    before = 1\n    {manager} manager(\n        1,\n        option=True,\n    ) as context:\n{body}{continuation}"
                );
                assert_matches_cpython(&source, "case.py", &LEVELS);
            }
        }
    }

    assert_matches_cpython(
        "def outer():\n    captured = 1\n    def f(m):\n        with m as token:\n            nonlocal captured\n            assert predicate()\n        return captured, token\n    return f\n",
        "case.py",
        &LEVELS,
    );
}

#[test]
fn modern_manager_finally_jump_and_copied_exit_keep_owners() {
    const JUMP_SOURCE: &str = "def f(manager, work):\n    try:\n        raise ValueError()\n    except ValueError:\n        with manager:\n            try:\n                work()\n            finally:\n                pass\n";
    const DIRECT_SOURCE: &str = "def f(manager, work):\n    with manager:\n        try:\n            work()\n        finally:\n            pass\n";
    const PREFIX_SOURCE: &str = "def f(manager, work):\n    with manager:\n        pass\n        try:\n            work()\n        finally:\n            pass\n";
    const COPY_SOURCE: &str = "def f(manager, work, flag):\n    try:\n        if flag:\n            return work()\n        work()\n    finally:\n        with manager:\n            try:\n                work()\n            finally:\n                pass\n";
    const ASSERT_SOURCE: &str = "def f(manager, work, flag):\n    with manager:\n        pass\n        try:\n            work()\n        finally:\n            pass\n            assert flag\n";
    const TRACE: &str = r"
import dis, marshal, sys
mode = sys.argv[1]
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if (i.opname == 'JUMP_FORWARD' and i.positions.lineno == 9 if mode == 'jump'
               else i.opname == 'NOT_TAKEN' and i.positions.lineno == 8 if mode == 'assert'
               else i.opname == 'NOP' and i.positions.lineno == (6 if mode == 'direct' else 3 if mode == 'prefix' else 11))]
if mode == 'assert':
    targets = targets[:1]
if (len(targets) < 2 if mode == 'copy' else len(targets) != 1):
    raise RuntimeError(('missing finalizer marker', mode, targets))
monitor = sys.monitoring
monitor.use_tool_id(5, 'modern-manager-finally-owner')
for flag in ((True, False) if mode == 'copy' else (False,)):
    target = targets[1 if mode == 'copy' and not flag else 0]
    for exception in (RuntimeError, StopIteration):
        for suppress in (False, True):
            events, hits = [], []
            class Manager:
                def __enter__(self):
                    events.append('enter')
                def __exit__(self, typ, value, traceback):
                    events.append(('exit', typ))
                    return suppress
            def work():
                events.append('work')
                return 7
            def inject(code, offset):
                if code is f.__code__ and offset == target:
                    hits.append(offset)
                    raise exception('injected')
            monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
            monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
            try:
                if mode in ('copy', 'assert'):
                    f(Manager(), work, flag)
                else:
                    f(Manager(), work)
                outcome = None
            except BaseException as error:
                outcome = type(error)
            finally:
                monitor.set_local_events(5, f.__code__, 0)
                monitor.register_callback(5, monitor.events.INSTRUCTION, None)
            if mode in ('jump', 'prefix') or (mode == 'assert' and sys.version_info[:2] == (3, 14)):
                expected_events = (['enter', 'work', ('exit', exception)] if mode in ('jump', 'assert')
                                   else ['enter', ('exit', exception)])
                expected_outcome = None if suppress else exception
            else:
                expected_events = (['enter', 'work'] if mode in ('direct', 'assert')
                                   else ['work', 'enter', 'work'])
                expected_outcome = exception
            if hits != [target] or events != expected_events or outcome is not expected_outcome:
                raise RuntimeError((mode, hits, events, outcome, expected_outcome))
monitor.free_tool_id(5)
";

    for (index, python_version) in [(2, PythonVersion::Py314), (3, PythonVersion::Py315)] {
        for optimization_level in LEVELS {
            for (mode, source) in [
                ("jump", JUMP_SOURCE),
                ("direct", DIRECT_SOURCE),
                ("prefix", PREFIX_SOURCE),
                ("copy", COPY_SOURCE),
                ("assert", ASSERT_SOURCE),
            ] {
                if mode == "assert" && optimization_level != OptimizationLevel::Zero {
                    continue;
                }
                let [native, actual] = compile_both(
                    source,
                    "modern_manager_finally.py",
                    python_version,
                    optimization_level,
                );
                assert_runs(
                    probe(index).args(["-I", "-S", "-B", "-c", TRACE, mode]),
                    [&native, &actual],
                );
            }
        }
    }
}

#[test]
fn legacy_manager_finally_jump_keeps_manager_owner() {
    const SOURCE: &str = "def f(manager, work):\n    try:\n        raise ValueError()\n    except ValueError:\n        with manager:\n            pass\n            try:\n                work()\n            finally:\n                pass\n";
    const TRACE: &str = r"
import dis, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'JUMP_FORWARD' and i.positions.lineno == 10]
if len(targets) != 1:
    raise RuntimeError(('missing finalizer jump', targets))
target = targets[0]
monitor = sys.monitoring
monitor.use_tool_id(5, 'legacy-manager-finally-owner')
for error_type in (RuntimeError, StopIteration):
    for suppress in (False, True):
        events, hits = [], []
        class Manager:
            def __enter__(self):
                events.append('enter')
            def __exit__(self, typ, value, traceback):
                events.append(('exit', typ))
                return suppress
        def work():
            events.append('work')
        def inject(code, offset):
            if offset == target:
                hits.append(offset)
                raise error_type('injected')
        monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
        monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
        try:
            f(Manager(), work)
            outcome = None
        except BaseException as error:
            outcome = type(error)
        finally:
            monitor.set_local_events(5, f.__code__, 0)
            monitor.register_callback(5, monitor.events.INSTRUCTION, None)
        expected = None if suppress else error_type
        if hits != [target] or events != ['enter', 'work', ('exit', error_type)] or outcome is not expected:
            raise RuntimeError((hits, events, outcome, expected))
monitor.free_tool_id(5)
";

    for (index, python_version) in [PythonVersion::Py312, PythonVersion::Py313]
        .into_iter()
        .enumerate()
    {
        for optimization_level in LEVELS {
            let [native, actual] = compile_both(
                SOURCE,
                "legacy_manager_finally.py",
                python_version,
                optimization_level,
            );
            assert_eq!(actual, native, "{python_version}, {optimization_level:?}");
            assert_runs(
                probe(index).args(["-I", "-S", "-B", "-c", TRACE]),
                [&native, &actual],
            );
        }
    }
}

#[test]
fn elided_prefix_preserves_finally_manager_exit() {
    let source = "def run(worker):\n    with outer():\n        with inner():\n            item = worker.read()\n            assert item\n            pending = worker.start()\n            try:\n                consume(pending)\n            finally:\n                worker.finish()\n            assert item\n";
    let without_prefix_assert = source.replacen("            assert item\n", "", 1);
    for (index, source) in [
        source.to_owned(),
        without_prefix_assert.clone(),
        source.replacen("            assert item\n", "            pass\n", 1),
        source.replacen(
            "            assert item\n",
            "            inspect(item)\n",
            1,
        ),
        without_prefix_assert.replace(
            "            pending = worker.start()\n",
            "            pending = worker.start()\n            assert item\n",
        ),
        source.replacen(
            "            assert item\n",
            "            assert item\n            assert worker\n",
            1,
        ),
        format!("{source}            inspect(item)\n"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("elided_prefix_finally_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn completed_prefix_preserves_finally_manager_exit() {
    for function in ["def", "async def"] {
        for prefix in [
            "        from package import item\n        saved = obj.value\n",
            "        with manager:\n            prepare()\n        saved = obj.value\n",
            "        with acquire() as resource:\n            prepare(resource)\n        from package import item\n        saved = obj.value\n",
        ] {
            let source = format!(
                "{function} f(manager, obj):\n    with manager:\n{prefix}        try:\n            result = obj.work()\n        finally:\n            obj.value = saved\n        assert result\n        assert obj\n"
            );
            assert_matches_cpython(&source, "completed_prefix_finally.py", &LEVELS);
        }
    }
}

#[test]
fn nested_definition_before_finally_preserves_manager_exit() {
    for definition in [
        "async def child():\n            yield flag",
        "async def child(*, item: Item) -> Items[Item]:\n            yield flag",
        "def child():\n            return flag",
    ] {
        let source = format!(
            "async def f(manager, task, flag):\n    with manager:\n        {definition}\n        configure(child)\n        try:\n            assert flag\n        finally:\n            release()\n            result = await task\n        assert result\n"
        );
        assert_matches_cpython(&source, "nested_definition_finally_exit.py", &LEVELS);
    }
    assert_matches_cpython(
        "async def f(manager, task, flag):\n    with manager:\n        'marker'\n        configure(flag)\n        try:\n            assert flag\n        finally:\n            release()\n            result = await task\n        assert result\n",
        "literal_prefix_finally_exit.py",
        &LEVELS,
    );
}

#[test]
fn elided_assertion_after_manager_finally_preserves_preceding_pass_owner() {
    const SOURCE: &str = "def f(cm, x):\n    with cm:\n        pass\n        try:\n            work()\n        finally:\n            cleanup()\n        assert x\n";
    const TRACE: &str = r"
import dis, json, marshal, sys
events = []
class Fault(Exception): pass
class Manager:
    def __enter__(self):
        events.append('enter')
    def __exit__(self, typ, value, tb):
        events.append(('exit', typ.__name__ if typ else None))
        return False
scope = {'work': lambda: events.append('work'), 'cleanup': lambda: events.append('cleanup')}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOP' and i.positions.lineno == 3]
if len(targets) != 1:
    raise RuntimeError('expected one authored pass NOP')
hits = []
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        frame.f_trace_opcodes = True
        if event == 'opcode' and frame.f_lasti == targets[0]:
            hits.append(True)
            raise Fault()
    return trace
sys.settrace(trace)
try:
    try:
        f(Manager(), True)
    except Fault:
        events.append('escaped')
finally:
    sys.settrace(None)
if len(hits) != 1:
    raise RuntimeError('authored pass NOP was not reached')
print(json.dumps(events))
";
    for (index, (python_version, _, _)) in VERSIONS.iter().copied().enumerate().skip(2) {
        for optimization_level in &LEVELS[1..] {
            let [native, actual] = compile_both(
                SOURCE,
                "manager_finally_assert.py",
                python_version,
                *optimization_level,
            );
            assert_outputs_match(
                probe(index).args(["-I", "-S", "-B", "-c", TRACE]),
                [&native, &actual],
            );
        }
    }
}

#[test]
fn empty_try_in_conditional_finally_compacts_setup_before_exit() {
    const SOURCE: &str = "async def f(manager, items, work, flag):\n    async with manager:\n        for item in items:\n            try:\n                await work(item)\n            finally:\n                if flag:\n                    value = work()\n                    count = value / 2\n                    try:\n                        assert count == item\n                    except AssertionError:\n                        pass\n";
    for body in ["assert count == item", "pass", "work(count)"] {
        let source = SOURCE.replace("assert count == item", body);
        assert_matches_cpython(&source, "empty_try_finally_exit.py", &LEVELS);
    }
}

#[test]
fn nested_erased_conditions_preserve_original_function_return() {
    let source = "async def f(state):\n    result = await prepare()\n    if state.previous is not None:\n        assert result\n    if state.outer is not None:\n        if state.inner is None:\n            assert result\n        else:\n            assert state.inner\n        assert result\n";
    for (index, source) in [
        source.to_owned(),
        source
            .replace("async def", "def")
            .replace("await prepare()", "prepare()"),
        source.replace(
            "    if state.previous is not None:\n        assert result\n",
            "",
        ),
        source.replace("state.outer is not None", "state.outer is None"),
        source.replace("state.inner is None", "state.inner is not None"),
        format!("{source}    else:\n        assert result\n"),
        source
            .replace("\n    ", "\n        ")
            .replacen('\n', "\n    with manager:\n", 1),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("nested_erased_return_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_suffix_preserves_original_function_return() {
    let source = "async def f(first, second):\n    await prepare()\n    if second:\n        assert second\n    if first and not second:\n        assert first\n    else:\n        assert second\n    assert first\n";
    for (index, source) in [
        source.to_owned(),
        source
            .replace("async def", "def")
            .replace("await prepare()", "prepare()"),
        source.replace("    if second:\n        assert second\n", ""),
        "async def f(first, second):\n    await prepare()\n    if second:\n        assert second\n    while first:\n        assert second\n    assert second\n".to_owned(),
        "async def f(first, second):\n    await prepare()\n    if second:\n        assert second\n    while first:\n        if second:\n            break\n        work()\n    assert second\n".to_owned(),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("erased_suffix_return_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn protected_erased_conditionals_preserve_repeated_join_predecessors() {
    let source = "async def f(manager, earlier, value, items):\n    async with manager:\n        await prepare()\n        if earlier:\n            assert ready()\n        if value is not None:\n            assert ready()\n        else:\n            assert other()\n        for item in items:\n            await consume(item)\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("async with manager:", "with manager:"),
        source.replace("            assert ready()", "            prepare()"),
        source.replace("        if earlier:\n            assert ready()\n", ""),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("protected_erased_join_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_conditionals_preserve_repeated_join_predecessors_in_suspended_scopes() {
    let source = "async def f(flag, items):\n    await prepare()\n    if other:\n        assert ready()\n    if flag:\n        assert ready()\n    else:\n        assert other()\n    value = consume(items)\n    for item in items:\n        assert item\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("    await prepare()\n", ""),
        source
            .replace("async def", "def")
            .replace("    await prepare()\n", ""),
        source
            .replace("async def", "def")
            .replace("    await prepare()", "    yield prepare()"),
        source.replace("        assert other()", "        consume(items)"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("erased_condition_join_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn empty_try_before_manager_cleanup_keeps_its_exit_unowned() {
    let source = "def f(manager):\n    try:\n        prepare()\n        with manager as resource:\n            resource.first()\n            resource.second()\n            try:\n                assert resource.check()\n            except Error:\n                pass\n        after()\n    finally:\n        finish()\n";
    for (index, source) in [
        source.to_owned(),
        source.replace(
            "                assert resource.check()",
            "                pass",
        ),
        source.replace(
            "                assert resource.check()",
            "                resource.check()",
        ),
        source.replace("        after()\n", ""),
        source.replace(
            "                pass\n",
            "                pass\n            resource.third()\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("empty_try_manager_cleanup_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn optimized_empty_try_loop_keeps_enclosing_exception_owner() {
    const TRACE_BACKEDGE: &str = r"
import dis, json, marshal, sys

events = []
def recover():
    events.append('recover')
def cleanup():
    events.append('cleanup')
def log():
    events.append('log')
scope = dict(recover=recover, cleanup=cleanup, log=log)
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
backedge = next(i.offset for i in dis.get_instructions(f) if i.opname == 'JUMP_BACKWARD')
fired = [False]
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        if event == 'call':
            frame.f_trace_opcodes = True
        elif event == 'opcode' and frame.f_lasti == backedge:
            fired[0] = True
            raise ValueError('injected')
    return trace
sys.settrace(trace)
# Prime opcode tracing, then reset it before the execution under test.
f([])
sys.settrace(None)
events.clear()
sys.settrace(trace)
try:
    f([1])
except ValueError:
    events.append('escaped')
finally:
    sys.settrace(None)
assert fired[0]
print(json.dumps(events))
";
    assert_matches_cpython(
        "async def f(xs):\n    for x in xs:\n        try:\n            assert x\n        except:\n            log()\n",
        "case.py",
        &LEVELS,
    );
    let cases = [
        (
            "def f(xs):\n    try:\n        for x in xs:\n            try:\n                assert x\n            except:\n                log()\n    except Exception:\n        recover()\n",
            &b"[\"recover\"]\n"[..],
        ),
        (
            "def f(xs):\n    try:\n        for x in xs:\n            try:\n                assert x\n            except:\n                log()\n    finally:\n        cleanup()\n",
            &b"[\"cleanup\", \"escaped\"]\n"[..],
        ),
    ];
    for (source, expected_events) in cases {
        for (index, (python_version, _, _)) in VERSIONS.iter().copied().enumerate() {
            for optimization_level in LEVELS {
                let [native, actual] =
                    compile_both(source, "case.py", python_version, optimization_level);
                let output = assert_outputs_match(
                    probe(index).args(["-I", "-S", "-c", TRACE_BACKEDGE]),
                    [&native, &actual],
                );
                assert_eq!(
                    output, expected_events,
                    "{python_version}, {optimization_level:?}"
                );
                assert_eq!(actual, native, "{python_version}, {optimization_level:?}");
            }
        }
    }
}

#[test]
fn optimized_match_guards_preserve_default_locations() {
    for (pattern, guard) in [("y", "__debug__"), ("_", "__debug__"), ("y", "False")] {
        let source = format!(
            "def f(x):\n    match x:\n        case {pattern} if {guard}:\n            return 1\n        case _:\n            return 2\n"
        );
        assert_matches_cpython(&source, "match_guard.py", &LEVELS);
    }

    for source in [
        r#"def f(x, flag, other):
    match x:
        case y if (
            __debug__
        ):
            raise RuntimeError("guard")
        case _:
            raise ValueError("default")
"#,
        r#"def f(x, manager, extra):
    try:
        trigger()
    except* ValueError as e:
        match x:
            case y if (
                __debug__
            ):
                raise RuntimeError("guard")
            case _:
                raise ValueError("default")
    return 9
"#,
        r#"def f(x, manager, extra):
    with manager:
        try:
            trigger()
        finally:
            match x:
                case y if (
                    __debug__
                ):
                    raise RuntimeError("guard")
                case _:
                    raise ValueError("default")
    return 9
"#,
    ] {
        assert_matches_cpython(source, "match_guard_control.py", &LEVELS);
    }
}

#[test]
fn elided_else_preserves_async_comprehension_exit() {
    for comprehension in [
        "[item async for item in items]",
        "{item async for item in items}",
        "{item: item async for item in items}",
    ] {
        for otherwise in ["assert flag", "value = items"] {
            for continuation in ["return await consume(value)", "return value"] {
                let source = format!(
                    "async def f(flag, items):\n    if flag:\n        value = {comprehension}\n    else:\n        {otherwise}\n    {continuation}\n"
                );
                assert_matches_cpython(&source, "async_comprehension_exit.py", &LEVELS);
            }
        }
    }
}

#[test]
fn async_comprehension_finalizer_preserves_conditional_owner() {
    for body in ["yield value", "await operation()"] {
        for comprehension in [
            "[item async for item in source]",
            "{item async for item in source}",
            "{item: item async for item in source}",
            "[item for item in source]",
        ] {
            let source = format!(
                "async def sample(manager, source, value):\n    async with manager:\n        try:\n            {body}\n        finally:\n            collected = {comprehension}\n            if collected:\n                await consume(*collected)\n"
            );
            assert_matches_cpython_versions(
                &source,
                "async_comprehension_finalizer.py",
                &LEVELS,
                &[
                    PythonVersion::Py312,
                    PythonVersion::Py313,
                    PythonVersion::Py314,
                    PythonVersion::Py315,
                ],
            );
        }
    }
}

#[test]
fn async_comprehension_preserves_conditional_exception_owner() {
    const TRACE_CONDITION: &str = r"
import dis, marshal, sys

scope = {'work': lambda: None}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOT_TAKEN' and i.positions.lineno == int(sys.argv[1])]
assert len(targets) == 1, targets
target = targets[0]
hits = []
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        frame.f_trace_opcodes = True
        if event == 'opcode' and frame.f_lasti == target:
            hits.append(frame.f_lasti)
            raise ValueError('injected')
    return trace
def drive(x):
    try:
        f(x, [('k', 1), ('j', 2)], {}).send(None)
    except StopIteration:
        return 'completed'
    except ValueError:
        return 'escaped'
    return 'suspended'
sys.settrace(trace)
warm = drive(False)
sys.settrace(None)
hits.clear()
sys.settrace(trace)
try:
    outcome = drive(True)
finally:
    sys.settrace(None)
assert warm == 'completed', warm
assert hits == [target], hits
print(outcome)
";
    let cases = [
        ("{**{k: 1 for k in xs}}", "", 4, false),
        ("[{k: 1 for k in xs}]", "", 4, false),
        ("[x, *[k for k in xs], x]", "", 4, false),
        ("({k: 1 for k in xs}, 1)", "", 4, false),
        ("(x, {k: 1 for k in xs})", "", 4, false),
        ("([x, *xs], [x, *[k for k in xs]])", "", 4, false),
        ("([x, *[k for k in xs]], [x, *xs])", "", 4, false),
        ("(*({k: 1 for k in xs},), x)", "", 4, false),
        ("([k for k in xs], (*xs, 1, 2))", "", 4, false),
        ("([k for k in xs], (*[1, 2], [x, *xs]))", "", 4, false),
        ("{**dict(xs)}", "", 4, true),
        ("{**{k: 1 for k in xs}}", "    pass\n", 5, false),
        ("{**{k: 1 for k in xs}}", "    pass; pass\n", 5, true),
        (
            "{**{k: 1 for k in xs}}",
            "    try:\n        work()\n    finally:\n        pass\n",
            8,
            false,
        ),
    ];
    for (expression, prefix, condition_line, protected_314) in cases {
        let source = format!(
            "async def f(x, xs, d):\n    v = {expression}\n{prefix}    try:\n        if x:\n            pass\n    except Exception:\n        pass\n"
        );
        for (index, (python_version, _, _)) in VERSIONS.iter().copied().enumerate() {
            for optimization_level in LEVELS {
                let [native, actual] =
                    compile_both(&source, "case.py", python_version, optimization_level);
                assert_eq!(
                    actual, native,
                    "{python_version}, {optimization_level:?}, {expression}"
                );
                if matches!(python_version, PythonVersion::Py314 | PythonVersion::Py315) {
                    let expected = if protected_314 && python_version == PythonVersion::Py314 {
                        &b"completed\n"[..]
                    } else {
                        &b"escaped\n"[..]
                    };
                    let output = assert_outputs_match(
                        probe(index)
                            .args(["-I", "-S", "-B", "-c", TRACE_CONDITION])
                            .arg(condition_line.to_string()),
                        [&native, &actual],
                    );
                    assert_eq!(
                        output, expected,
                        "{python_version}, {optimization_level:?}, {expression}"
                    );
                }
            }
        }
    }
}

#[test]
fn discarded_async_yield_preserves_comprehension_exception_owners() {
    for prefix in [
        "    assert value\n    assert obj\n",
        "",
        "    pass\n",
        "    consume(value)\n",
        "    yield value\n",
        "    await later()\n",
    ] {
        let source = format!(
            "async def f(obj, value):\n    value = {{**{{key: 0 for key in obj}}, \"a\": 0, \"b\": 0}}\n    yield value\n{prefix}    try:\n        if obj.flag:\n            consume(value)\n        yield value\n        await later()\n    except Exception:\n        handle()\n"
        );
        assert_matches_cpython(&source, "discarded_async_yield.py", &LEVELS);
    }
}

#[test]
fn comprehension_collection_continuations_preserve_exception_owners() {
    for expression in [
        "[x, *[k for k in xs], x]",
        "({k: 1 for k in xs}, 1)",
        "([x, *xs], [x, *[k for k in xs]])",
        "(*({k: 1 for k in xs},), x)",
        "([k for k in xs], (*[1, 2], [x, *xs]))",
    ] {
        let source = format!(
            "def f(x, xs, d):\n    v = {expression}\n    try:\n        if x:\n            pass\n    except Exception:\n        pass\n"
        );
        assert_matches_cpython(&source, "case.py", &LEVELS);
    }
}

#[test]
fn elided_assertion_identifiers_preserve_frozenset_identity() {
    const SOURCE: &str = r#"
def first():
    sink(items={"shared_member", "second_member", "third_member"})

def second():
    sink(extra=None, items={"shared_member", "second_member", "third_member"})

def discarded_assert():
    assert context.shared_member
"#;
    const TEXT_ORACLE: &str = r"
import marshal, sys
source = sys.stdin.buffer.read().decode('utf-8')
code = compile(source, sys.argv[2], 'exec', dont_inherit=True, optimize=int(sys.argv[1]))
sys.stdout.buffer.write(marshal.dumps(code))
";
    const IDENTITY_ORACLE: &str = r"
import marshal, sys, types
module = marshal.loads(sys.stdin.buffer.read())
functions = {c.co_name: c for c in module.co_consts if isinstance(c, types.CodeType)}
sets = [next(c for c in functions[name].co_consts if isinstance(c, frozenset))
        for name in ('first', 'second')]
assert sets[0] == sets[1]
assert (sets[0] is sets[1]) == (sys.argv[1] == 'shared')
";
    let cases = [
        (SOURCE.to_string(), false),
        (SOURCE.replace("    assert context.shared_member", "    pass"), true),
        (SOURCE.replace("    assert context.shared_member", "    context.shared_member"), false),
        (SOURCE.replace("extra=None, ", ""), true),
        (SOURCE.replace("\"shared_member\"", "\"Key_member\"").replace("context.shared_member", "context.\u{212a}ey_member"), false),
        (SOURCE.replace("assert context.shared_member", "assert (lambda: context.shared_member)()"), false),
        (SOURCE.replace("shared_member", "_Box__member").replace("def discarded_assert():\n    assert context._Box__member", "class Box:\n    def f(self):\n        return self.__member\n\n    def discarded_assert(self):\n        assert context._Box__member"), false),
        (SOURCE.replace("def discarded_assert():\n    assert context.shared_member", "assert context.shared_member"), false),
    ];
    for (source, shared) in cases {
        for (version, _, _) in VERSIONS {
            for level in [OptimizationLevel::One, OptimizationLevel::Two] {
                let options = CompileOptions {
                    python_version: version,
                    optimization_level: level,
                    ..CompileOptions::default()
                };
                let expected = oracle(source.as_bytes(), "case.py", version, level);
                assert!(
                    expected.status.success(),
                    "{}",
                    String::from_utf8_lossy(&expected.stderr)
                );
                let actual = compile_bytes_with_options(source.as_bytes(), "case.py", &options)
                    .unwrap()
                    .marshal();
                assert_eq!(actual, expected.stdout, "{version}, {level:?}, bytes");

                let index = VERSIONS
                    .iter()
                    .position(|(candidate, _, _)| *candidate == version)
                    .unwrap();
                let level_arg = if level == OptimizationLevel::One {
                    "1"
                } else {
                    "2"
                };
                let expected_text = run_successfully(
                    probe(index)
                        .args([
                            "-I",
                            "-S",
                            "-X",
                            "utf8",
                            "-c",
                            TEXT_ORACLE,
                            level_arg,
                            "case.py",
                        ])
                        .stdout(Stdio::piped()),
                    source.as_bytes(),
                );
                let actual_text = compile_with_options(&source, "case.py", &options)
                    .unwrap()
                    .marshal();
                assert_eq!(actual_text, expected_text, "{version}, {level:?}, text");

                assert_runs(
                    probe(index)
                        .args([
                            "-I",
                            "-S",
                            "-X",
                            "utf8",
                            "-c",
                            IDENTITY_ORACLE,
                            if shared { "shared" } else { "distinct" },
                        ])
                        .stdout(Stdio::piped()),
                    [&expected.stdout, &actual],
                );
            }
        }
    }
}

#[test]
fn elided_non_ascii_identifier_preserves_discarded_builders() {
    const PREFIX: &str = "def marker():\n    assert probe.élève\n\n";
    let discarded = [
        "if False:\n    def hidden(value):\n        return value + 1\n",
        "saved = (\"élève\",)\nif False:\n    class Hidden:\n        def method(self):\n            return (\"élève\",)\n",
        "if False:\n    async def hidden(items):\n        async for item in items:\n            try:\n                yield item\n            finally:\n                cleanup()\n",
    ];
    for body in discarded {
        assert_matches_cpython(&format!("{PREFIX}{body}"), "case.py", &LEVELS);
    }
}

#[test]
fn comprehension_branches_preserve_backedge_store_locations() {
    let source = "def check_groups(groups, flag):\n    for group in groups:\n        if flag:\n            selected = [item for item in group if ready(item)]\n            assert selected\n        else:\n            selected = [item for item in group if item.active]\n            assert selected\n        assert len(selected) == 1\n";
    for source in [
        source.to_owned(),
        source.replace("[item for item in group", "[index for index, item in enumerate(group)"),
        source.replace("[item for item in group", "[index for index, item, extra in triples(group)"),
        source.replace("[item for item in group", "[(item, other) for item in group for other in group"),
        source.replace("[item for item in group if item.active]", "[\n                item\n                for item in group\n                if item.active\n            ]"),
        source.replace("[item for item in group if ready(item)]", "{item for item in group if ready(item)}").replace("[item for item in group if item.active]", "{item for item in group if item.active}"),
        source.replace("[item for item in group if ready(item)]", "{item: item for item in group if ready(item)}").replace("[item for item in group if item.active]", "{item: item for item in group if item.active}"),
        source.replace("selected =", "item ="),
        source.replace("selected =", "flag.selected ="),
        source.replace("        assert len(selected) == 1", "        consume(selected)"),
    ] {
        assert_matches_cpython(&source, "comprehension_backedge_store.py", &LEVELS);
    }
}

#[test]
fn elided_assertions_after_comprehension_branches_keep_store_return_locations() {
    for prefix in ["", "async "] {
        for comprehension in [
            "[item for item in items]",
            "{item for item in items}",
            "{item: item for item in items}",
            "[\n            item\n            for item in items\n        ]",
        ] {
            for branch_tail in ["", "        assert result\n"] {
                let source = format!(
                    "{prefix}def f(items, flag):\n    if flag:\n        result = {comprehension}\n{branch_tail}    assert flag\n"
                );
                assert_matches_cpython(&source, "comprehension_branch_return.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_branch_assertions_preserve_comprehension_store_return_locations() {
    for scope in ["def", "async def"] {
        for comprehension in [
            "[item for item in items]",
            "{item for item in items if item.active}",
            "{item: item.value for item in items}",
        ] {
            for prefix in [
                "",
                "        consume(items)\n",
                "        value = consume(items)\n",
            ] {
                for branch_tail in ["", "        assert out\n        assert items\n"] {
                    for suffix in ["", "    assert flag\n"] {
                        let source = format!(
                            "{scope} f(flag, items):\n    if flag:\n{prefix}        out = {comprehension}\n{branch_tail}    else:\n{prefix}        out = {comprehension}\n{branch_tail}{suffix}"
                        );
                        assert_matches_cpython(&source, "erased_comprehension_footer.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn short_circuit_prefix_preserves_nested_return_donors() {
    let source = r"def validate(kind):
    if kind == 1:
        enabled = ready() and first() in (1, 2)
        assert enabled
        active = ready()
        if active:
            left = first()
            right = second()
            assert left
            assert right
        else:
            assert enabled
            left = first() if ready() else 1
        assert left
    else:
        raise ValueError(kind)
";
    for source in [
        source.to_owned(),
        source.replace(
            "ready() and first() in (1, 2)",
            "ready() or first() in (1, 2)",
        ),
        source.replace(
            "ready() and first() in (1, 2)",
            "(ready() or other()) and first() in (1, 2)",
        ),
        source.replace("ready() and first() in (1, 2)", "ready()"),
    ] {
        assert_matches_cpython(&source, "short_circuit_prefix_return.py", &LEVELS);
    }
}

#[test]
fn suspended_comprehension_siblings_preserve_return_locations() {
    let source = r"async def collect(flag, items):
    if flag:
        with manager():
            result = [item async for item in items]
    else:
        result = [item async for item in items]
    assert result
";
    for source in [
        source.to_owned(),
        source.replace(
            "[item async for item in items]",
            "{item async for item in items}",
        ),
        source.replace(
            "[item async for item in items]",
            "{item: item async for item in items}",
        ),
        source.replace(
            "[item async for item in items]",
            "[await transform(item) for item in items]",
        ),
        source.replace("async for", "for"),
        source.replace("assert result", "consume(result)"),
    ] {
        assert_matches_cpython(&source, "suspended_comprehension_siblings.py", &LEVELS);
    }
}

#[test]
fn coroutine_comprehension_siblings_preserve_return_locations() {
    let source = "async def collect(flag, items, fields):\n    await prepare()\n    if flag:\n        with manager():\n            result = [tuple(item for field in fields) for item in items]\n    else:\n        result = [\n            tuple(item for field in fields)\n            for item in items\n        ]\n    assert result\n";
    for source in [
        source.to_owned(),
        source.replace("tuple(item for field in fields)", "item"),
        source.replace(
            "with manager():",
            "with manager() as active, second_manager() as other:",
        ),
        source.replace("    assert result\n", "    consume(result)\n"),
        "async def collect(flag, items):\n    await prepare()\n    if flag:\n        work()\n    else:\n        result = [item for item in items]\n    assert flag\n".to_owned(),
        "async def collect(flag, other, items):\n    await prepare()\n    if flag:\n        result = [item for item in items]\n    elif other:\n        result = {item for item in items}\n    else:\n        result = {item: item for item in items}\n    assert result\n".to_owned(),
    ] {
        assert_matches_cpython(&source, "coroutine_comprehension_siblings.py", &LEVELS);
    }
}

#[test]
fn chained_generator_loop_tail_preserves_stop_iteration_wrapper() {
    const SOURCE: &str = "def test(xs, y, z):\n    yield 0\n    for x in xs:\n        if x:\n            h()\n        elif x < y < z:\n            yield x\n        else:\n            assert z\n";
    const TRACE_ORACLE: &str = r"
import dis, marshal, sys
namespace = {'h': lambda: None}
exec(marshal.loads(sys.stdin.buffer.read()), namespace)
function = namespace['test']
instructions = list(dis.get_instructions(function))
targets = [instructions[i].offset for i in range(1, len(instructions))
           if instructions[i].opname == 'JUMP_BACKWARD'
           and instructions[i - 1].opname == 'POP_TOP'
           and instructions[i].positions.lineno == 6]
assert len(targets) == 1, targets
generator = function([0], -1, 3)
assert next(generator) == 0

def trace(frame, event, arg):
    if frame.f_code is function.__code__:
        frame.f_trace_opcodes = True
        if event == 'opcode' and frame.f_lasti == targets[0]:
            raise StopIteration('trace interruption')
    return trace

sys.settrace(trace)
try:
    next(generator)
except RuntimeError as exc:
    assert isinstance(exc.__cause__, StopIteration), exc.__cause__
else:
    raise AssertionError('exception was not propagated')
finally:
    sys.settrace(None)
";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for level in [OptimizationLevel::One, OptimizationLevel::Two] {
            let expected = oracle(SOURCE.as_bytes(), "generator_chain.py", version, level);
            assert!(expected.status.success(), "{version}, {level:?}");
            let actual = compile_with_options(
                SOURCE,
                "generator_chain.py",
                &CompileOptions {
                    python_version: version,
                    optimization_level: level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            let index = VERSIONS
                .iter()
                .position(|(candidate, _, _)| *candidate == version)
                .unwrap();
            assert_runs(
                probe(index)
                    .args(["-I", "-S", "-X", "utf8", "-c", TRACE_ORACLE])
                    .stdout(Stdio::piped()),
                [&expected.stdout, &actual],
            );
        }
    }
}

#[test]
fn protected_loop_elided_else_excludes_conditional_cleanup() {
    const SOURCE: &str = "def test(xs, manager, flag):\n    try:\n        for x in xs:\n            if x:\n                with manager as value:\n                    pass\n                    pass\n                    global marker\n            else:\n                assert flag\n    finally:\n        cleanup()\n    return\n";
    const TRACE_ORACLE: &str = r"
import contextlib, dis, marshal, sys
class Interrupted(Exception):
    pass

events = []
namespace = {'cleanup': lambda: events.append('cleanup')}
exec(marshal.loads(sys.stdin.buffer.read()), namespace)
function = namespace['test']
targets = [instruction.offset for instruction in dis.get_instructions(function)
           if instruction.positions.lineno == 4
           and instruction.opname.startswith('POP_JUMP')]
assert len(targets) == 1, targets
for value in (False, True):
    hit = False
    def trace(frame, event, arg):
        global hit
        if frame.f_code is function.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == targets[0]:
                hit = True
                raise Interrupted()
        return trace
    sys.settrace(trace)
    try:
        function([value], contextlib.nullcontext(), True)
    except Interrupted:
        pass
    else:
        raise AssertionError('exception was not propagated')
    finally:
        sys.settrace(None)
    assert hit, 'conditional was not executed'
    assert not events, events
";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for level in [OptimizationLevel::One, OptimizationLevel::Two] {
            let expected = oracle(SOURCE.as_bytes(), "protected_loop.py", version, level);
            assert!(expected.status.success(), "{version}, {level:?}");
            let actual = compile_with_options(
                SOURCE,
                "protected_loop.py",
                &CompileOptions {
                    python_version: version,
                    optimization_level: level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            assert_eq!(actual, expected.stdout, "{version}, {level:?}");
            let index = VERSIONS
                .iter()
                .position(|(candidate, _, _)| *candidate == version)
                .unwrap();
            assert_runs(
                probe(index)
                    .args(["-I", "-S", "-X", "utf8", "-c", TRACE_ORACLE])
                    .stdout(Stdio::piped()),
                [&expected.stdout, &actual],
            );
        }
    }
}

#[test]
fn nested_try_fallthrough_preserves_conditional_exception_owners() {
    const EXACT: &str = "def f(a,b,c,d):\n    while c:\n        try:\n            try:\n                if a:\n                    pass\n            except Exception:\n                pass\n        finally:\n            x = 0\n        break\n";
    const FRESH: &str = "def f(a,b,c,d):\n    while c:\n        try:\n            try:\n                if a:\n                    pass\n            except Exception:\n                pass\n        finally:\n            d.clear()\n        break\n";
    const RETAINED: &str = "def f(a,b,c,d):\n    while c:\n        try:\n            try:\n                pass; pass\n                if a:\n                    pass\n            except Exception:\n                pass\n        finally:\n            d.clear()\n        break\n";
    const TRACE_CONDITION: &str = r"
import dis, marshal, sys

events = []
class Injected(Exception):
    pass
class Clean:
    def clear(self):
        events.append('cleanup')
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOT_TAKEN' and i.positions.lineno == int(sys.argv[1])]
assert len(targets) == 1, targets
target = targets[0]
hits = []
monitor = sys.monitoring
monitor.use_tool_id(5, 'conditional-owner')
def inject(code, offset):
    if offset == target:
        hits.append(offset)
        raise Injected()
monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
try:
    f(True, False, True, Clean())
    outcome = 'returned'
except Injected:
    outcome = 'escaped'
finally:
    monitor.set_local_events(5, f.__code__, 0)
    monitor.register_callback(5, monitor.events.INSTRUCTION, None)
    monitor.free_tool_id(5)
assert hits == [target], hits
print(outcome, events)
";

    assert_matches_cpython(EXACT, "fallthrough_nested_try.py", &LEVELS);
    for (source, line, expected) in [
        (FRESH, 5, &b"escaped []\n"[..]),
        (RETAINED, 6, &b"returned ['cleanup']\n"[..]),
    ] {
        for optimization_level in LEVELS {
            let [native, actual] = compile_both(
                source,
                "fallthrough_nested_try.py",
                PythonVersion::Py314,
                optimization_level,
            );
            let output = assert_outputs_match(
                probe(2)
                    .args(["-I", "-S", "-B", "-c", TRACE_CONDITION])
                    .arg(line.to_string()),
                [&native, &actual],
            );
            assert_eq!(output, expected, "{optimization_level:?}, line {line}");
        }
    }
}

#[test]
fn terminal_matches_before_erased_assertions_keep_case_returns() {
    for (case, source) in [
        "def f(a, b):\n    match a:\n        case 1:\n            if b:\n                action()\n            assert b\n    assert a\n",
        "async def f(a, b):\n    match a:\n        case 1:\n            if b:\n                action()\n            assert b\n    assert a\n",
        "async def f(a, b):\n    await action()\n    match a:\n        case 1:\n            if (value := action()):\n                consume(value)\n            assert b\n        case 2:\n            action()\n    assert a\n",
        "async def f(a, b):\n    match a:\n        case 1:\n            action()\n        case _:\n            if b:\n                action()\n            assert b\n    assert a\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("terminal_match_before_assertions_{case}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn nonlinear_wildcard_match_tails_preserve_false_edge_returns() {
    for prefix in ["def", "async def"] {
        for (case, tail) in [
            "assert b",
            "global token",
            "global token\n            assert b",
            "assert b\n            global token",
            "value: int",
            "(value): int",
            "nonlocal token",
        ]
        .into_iter()
        .enumerate()
        {
            let source = format!(
                "{prefix} f(a, b, c):\n    match a:\n        case 1:\n            action()\n        case _:\n            if b:\n                if c:\n                    action()\n                action()\n            {tail}\n    assert a\n"
            );
            let source = if tail == "nonlocal token" {
                format!(
                    "def outer():\n    token = 0\n    {}",
                    source.replace('\n', "\n    ")
                )
            } else {
                source
            };
            assert_matches_cpython_versions(
                &source,
                &format!("nonlinear_wildcard_match_tail_{case}.py"),
                &LEVELS,
                &[PythonVersion::Py314, PythonVersion::Py315],
            );
        }
    }
}

#[test]
fn nonlinear_match_cases_keep_their_shared_exits() {
    for (case, source) in [
        "def f(a, b, c, d, obj, items, cm):\n    match a:\n        case 1:\n            action()\n        case _:\n            if b:\n                if c:\n                    action()\n                action()\n            while b:\n                action()\n            assert b\n    assert a\n",
        "async def f(a, b, c, d, obj, items, cm):\n    match a:\n        case 1:\n            action()\n        case _:\n            if b:\n                if c:\n                    action()\n                action()\n            while b:\n                action()\n            assert b\n    assert a\n",
        "async def f(a, b, c, d, obj, items, cm):\n    match a:\n        case 1:\n            if a:\n                value = 1\n            else:\n                if b:\n                    if c:\n                        value = 2\n                    else:\n                        value = 3\n                else:\n                    value = 4\n                assert value\n    assert a\n",
        "async def f(a, b, c, d, obj, items, cm):\n    match a:\n        case 1:\n            action()\n        case _:\n            if a:\n                value = 1\n            else:\n                if b:\n                    if c:\n                        value = 2\n                    else:\n                        value = 3\n                else:\n                    value = 4\n                assert value\n    assert a\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("nonlinear_match_case_{case}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn completed_managers_preserve_coroutine_match_returns() {
    for manager in [
        "with manager as stream:\n                saved = stream.read()",
        "with manager:\n                saved = read()",
    ] {
        for suffix in ["assert value", "pass", "after()"] {
            let source = format!(
                "async def f(kind, manager, value):\n    match kind:\n        case 1:\n            {manager}\n            if change := compare(saved, value):\n                consume(change)\n            assert change == []\n        case 2:\n            with manager as stream:\n                stream.write(value)\n    {suffix}\n"
            );
            assert_matches_cpython(&source, "completed_manager_match.py", &LEVELS);
        }
    }
}

#[test]
fn nested_match_arms_preserve_tuple_return_copy_budget() {
    for otherwise in ["assert False", "pass", "result = [7]; tag = 'seven'"] {
        for returned in ["result, tag", "result", "result, tag, kind"] {
            let source = format!(
                "def f(kind, first, second):\n    tag = 'unset'\n    match kind:\n        case None:\n            if first:\n                if second:\n                    result = [1]\n                    tag = 'one'\n                elif first:\n                    result = [2]\n                    tag = 'two'\n                else:\n                    result = [3]\n                    tag = 'three'\n            elif second:\n                result = [4]\n                tag = 'four'\n            else:\n                result = [5]\n                tag = 'five'\n        case 'other':\n            result = [6]\n            tag = 'six'\n        case _:\n            {otherwise}\n    return {returned}\n"
            );
            assert_matches_cpython(&source, "nested_match_return_budget.py", &LEVELS);
        }
    }
}

#[test]
fn coroutine_match_cases_preserve_nested_condition_return_donors() {
    for prefix in [
        "",
        "    with cm:\n        with obj:\n            async for item in items:\n                action()\n            assert items\n",
    ] {
        for suffix in ["", "    assert a\n", "    after()\n"] {
            let source = format!(
                "async def f(a, b, c, obj, items, cm):\n{prefix}    match a:\n        case 1:\n            if b:\n                if c:\n                    action()\n                action()\n            assert b\n{suffix}"
            );
            assert_matches_cpython(&source, "nested_match_returns.py", &LEVELS);
        }
    }
}

#[test]
fn terminal_match_case_conditionals_keep_returns_before_erased_assertions() {
    for (case, source) in [
        "def f(a, b):\n    match a:\n        case 1:\n            if b:\n                action()\n            assert b\n",
        "def f(a, b):\n    match a:\n        case 1:\n            if (value := action()):\n                consume(value)\n            assert b\n        case 2:\n            with manager():\n                action()\n",
        "async def f(a, b):\n    match a:\n        case 1:\n            if b:\n                action()\n            assert b\n",
        "def f(a, b):\n    match a:\n        case 1:\n            action()\n        case _:\n            if b:\n                action()\n            assert b\n",
        "def f(a, b):\n    match a:\n        case 1 | 2:\n            if b:\n                action()\n            assert b\n        case _:\n            pass\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("terminal_match_case_{case}.py"),
            &LEVELS,
            &VERSIONS.map(|(version, _, _)| version),
        );
    }
}

#[test]
fn optimized_with_assertion_join_before_call_matches_cpython() {
    for condition in ["x", "not x", "x and y", "x < y", "f(x)", "x is None"] {
        let source = format!(
            "def f(x, y, callback, manager):\n    with manager:\n        if {condition}:\n            assert x\n        else:\n            assert y\n        callback(x)\n"
        );
        assert_matches_cpython_versions(
            &source,
            "optimized_with_join.py",
            &LEVELS,
            &VERSIONS.map(|(version, _, _)| version),
        );
    }

    assert_matches_cpython_versions(
        "def f(x, y, callback, manager1, manager2):\n    with manager1, manager2:\n        if y:\n            if x:\n                assert x\n            else:\n                assert y\n            callback(x if y else x)\n",
        "optimized_with_nested_join.py",
        &LEVELS,
        &[PythonVersion::Py314],
    );
}

#[test]
fn nested_pass_exit_retains_enclosing_try_owner() {
    const SOURCE: &str = "def f(flag, items):\n    try:\n        try:\n            pass\n        except ValueError:\n            pass\n        work()\n    except Fault:\n        events.append('caught')\n";
    const TRACE: &str = r"
import dis, marshal, sys
class Fault(Exception):
    pass
events = []
scope = {'Fault': Fault, 'events': events}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOP' and i.positions.lineno == 4]
assert len(targets) == 1, targets
target = targets[0]
hits = []
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        frame.f_trace_opcodes = True
        if event == 'opcode' and frame.f_lasti == target:
            hits.append(target)
            raise Fault()
    return trace
sys.settrace(trace)
try:
    f(True, [1])
finally:
    sys.settrace(None)
assert hits == [target], hits
assert events == ['caught'], events
";
    for (version, index) in [(PythonVersion::Py314, 2), (PythonVersion::Py315, 3)] {
        for optimization_level in LEVELS {
            let [native, actual] =
                compile_both(SOURCE, "nested_pass_exit.py", version, optimization_level);
            assert_eq!(actual, native, "{version} {optimization_level:?}");
            assert_runs(
                probe(index).args(["-I", "-S", "-c", TRACE]),
                [&native, &actual],
            );
        }
    }
}

#[test]
fn nested_pass_exit_successors_match_cpython() {
    for successor in ["out = 1", "del item", "if flag:\n            assert flag"] {
        let source = format!(
            "def f(flag, item):\n    try:\n        try:\n            pass\n        except ValueError:\n            pass\n        {successor}\n    except Fault:\n        events.append('caught')\n"
        );
        assert_matches_cpython_versions(
            &source,
            "nested_pass_successor.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }

    assert_matches_cpython_versions(
        "def f():\n    try:\n        try: pass\n        except ValueError:\n            pass\n        work()\n    except Fault:\n        events.append('caught')\n",
        "nested_same_line_pass.py",
        &LEVELS,
        &[PythonVersion::Py314, PythonVersion::Py315],
    );
}

#[test]
fn nonterminal_pass_finally_preserves_handler_for_all_targets() {
    for (function, trailer) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield 1\n"),
        ("async def", "    yield 1\n"),
    ] {
        let source = format!(
            "{function} f():\n    try:\n        try:\n            work()\n        finally:\n            pass\n        after()\n    except:\n        recover()\n{trailer}"
        );
        assert_matches_cpython(&source, "nonterminal_pass_finally.py", &LEVELS);
    }
}

#[test]
fn optimized_with_assertion_join_before_noncall_matches_cpython() {
    for successor in [
        "",
        "        value = 1\n",
        "        del flag\n",
        "        return flag\n",
        "        assert flag\n",
        "        if flag:\n            value = 1\n",
    ] {
        let source = format!(
            "def f(flag, manager):\n    with manager:\n        if flag:\n            assert flag\n        else:\n            assert flag\n{successor}"
        );
        assert_matches_cpython_versions(
            &source,
            "optimized_with_join_noncall.py",
            &LEVELS,
            &[PythonVersion::Py314],
        );
    }
    // The short-circuit edge also reaches this join and must not retain the NOP.
    assert_matches_cpython_versions(
        "def f(a, b, manager):\n    with manager:\n        if a or b:\n            assert a\n        else:\n            assert b\n        value = a\n",
        "optimized_with_shared_join.py",
        &LEVELS,
        &[PythonVersion::Py314],
    );
}

#[test]
fn optimized_with_assertion_join_before_noncall_keeps_manager_owner() {
    const SOURCE: &str = r"
def terminal(flag):
    with manager('terminal'):
        if flag:
            assert flag
        else:
            assert flag

def assigned(flag):
    with manager('outer'), manager('inner'):
        if flag: assert flag
        else: assert flag
        value = 1

def deleted(flag):
    with manager('delete'):
        if flag:
            assert flag
        else:
            assert flag
        del flag
";
    const TRACE: &str = r"
import dis, marshal, sys

events = []
class Fault(Exception):
    pass
class Manager:
    def __init__(self, name):
        self.name = name
    def __enter__(self):
        events.append((self.name, 'enter'))
    def __exit__(self, typ, value, tb):
        events.append((self.name, 'exit', typ.__name__ if typ else None))
        return False
scope = {'manager': Manager}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
for name, expected in [
    ('terminal', [('terminal', 'enter'), 'hit', ('terminal', 'exit', 'Fault')]),
    ('assigned', [('outer', 'enter'), ('inner', 'enter'), 'hit',
                  ('inner', 'exit', 'Fault'), ('outer', 'exit', 'Fault')]),
    ('deleted', [('delete', 'enter'), 'hit', ('delete', 'exit', 'Fault')]),
]:
    fn = scope[name]
    instructions = list(dis.get_instructions(fn))
    offsets = [b.offset for a, b in zip(instructions, instructions[1:])
               if a.opname == 'NOT_TAKEN' and b.opname == 'NOP']
    assert len(offsets) == 1, (name, offsets)
    events.clear()
    def trace(frame, event, arg):
        if frame.f_code is fn.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == offsets[0]:
                events.append('hit')
                raise Fault()
        return trace
    sys.settrace(trace)
    try:
        try:
            fn(True)
            raise AssertionError('injection did not escape')
        except Fault:
            pass
    finally:
        sys.settrace(None)
    assert events == expected, (name, events, expected)
";
    for level in [OptimizationLevel::One, OptimizationLevel::Two] {
        let [native, actual] = compile_both(
            SOURCE,
            "optimized_with_join_owner.py",
            PythonVersion::Py314,
            level,
        );
        assert_eq!(actual, native);
        let index = VERSIONS
            .iter()
            .position(|(version, _, _)| *version == PythonVersion::Py314)
            .unwrap();
        assert_runs(
            probe(index).args(["-I", "-S", "-c", TRACE]),
            [&native, &actual],
        );
    }
}

#[test]
fn protected_with_elided_if_keeps_manager_join_and_exception_owner() {
    const SOURCE: &str = r"
def f(flag, manager):
    try:
        with manager:
            if flag:
                assert flag
            else:
                assert not flag
        work()
    except Fault:
        recover()

def with_successor(flag, manager):
    try:
        with manager:
            if flag:
                assert flag
            else:
                assert not flag
            work()
    except Fault:
        recover()

def reverse(flag, manager):
    with manager:
        try:
            if flag:
                assert flag
            else:
                assert flag
        except Fault:
            recover()

def nested_reverse(flag, manager):
    with manager:
        try:
            try:
                if flag:
                    assert flag
                else:
                    assert flag
            except Fault:
                recover()
        except TypeError:
            work()

def except_star(flag, manager):
    try:
        with manager:
            if flag:
                assert flag
            else:
                assert not flag
    except* Fault:
        recover()
";
    const TRACE: &str = r"
import dis, marshal, sys

events = []
class Fault(Exception):
    pass
class Manager:
    def __enter__(self):
        events.append('enter')
    def __exit__(self, typ, value, tb):
        events.append(('exit', typ.__name__ if typ else None))
        return False
scope = {'Fault': Fault, 'work': lambda: events.append('work'),
         'recover': lambda: events.append('recover')}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
for name, flag, expected in (
    ('f', True, ['enter', 'hit', ('exit', 'Fault'), 'recover']),
    ('with_successor', True, ['enter', 'hit', ('exit', 'Fault'), 'recover']),
    ('reverse', False, ['enter', 'hit', ('exit', 'Fault'), 'escaped']),
    ('nested_reverse', False, ['enter', 'hit', ('exit', 'Fault'), 'escaped']),
    ('except_star', True, ['enter', 'hit', ('exit', 'Fault'), 'recover']),
):
    f = scope[name]
    events.clear()
    instructions = list(dis.get_instructions(f))
    line = next(i.positions.lineno for i in instructions if i.opname == 'POP_JUMP_IF_FALSE')
    offsets = [i.offset for i in instructions if i.opname == 'NOP' and i.positions.lineno == line]
    assert len(offsets) == 1, (name, offsets)
    def trace(frame, event, arg):
        if frame.f_code is f.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == offsets[0]:
                events.append('hit')
                raise Fault()
        return trace
    sys.settrace(trace)
    try:
        try:
            f(flag, Manager())
        except Fault:
            events.append('escaped')
    finally:
        sys.settrace(None)
    assert events == expected, (name, events, expected)
";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        assert_matches_cpython_versions(SOURCE, "protected_with_join.py", &LEVELS, &[version]);
        for level in [OptimizationLevel::One, OptimizationLevel::Two] {
            let [native, actual] = compile_both(SOURCE, "protected_with_join.py", version, level);
            assert_eq!(actual, native);
            let index = VERSIONS
                .iter()
                .position(|(candidate, _, _)| *candidate == version)
                .unwrap();
            assert_runs(
                probe(index).args(["-I", "-S", "-c", TRACE]),
                [&native, &actual],
            );
        }
    }
}

#[test]
fn protected_with_assertion_join_pass_has_no_exception_owner() {
    const SOURCE: &str = concat!(
        "def f(flag, manager):\n    try:\n        with manager:\n            if flag:\n                assert flag\n            else:\n                assert not flag\n            pass\n        work()\n    except Fault:\n        recover()\n",
        "\ndef g(flag, outer, inner):\n    try:\n        with outer:\n            with inner:\n                if flag:\n                    assert flag\n                else:\n                    assert not flag\n                pass\n            work()\n    except Fault:\n        recover()\n",
    );
    const TRACE: &str = r"
import dis, marshal, sys
events = []
class Fault(Exception):
    pass
class Manager:
    def __enter__(self):
        events.append('enter')
    def __exit__(self, typ, value, tb):
        events.append('exit')
        return False
scope = {'Fault': Fault, 'work': lambda: events.append('work'),
         'recover': lambda: events.append('recover')}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
for name, offset, args, expected in [
    ('f', 7, (True, Manager()), ['enter', 'hit']),
    ('g', 8, (True, Manager(), Manager()), ['enter', 'enter', 'hit']),
]:
    fn = scope[name]
    offsets = [instruction.offset for instruction in dis.get_instructions(fn)
               if instruction.opname == 'NOP'
               and instruction.positions.lineno == fn.__code__.co_firstlineno + offset]
    if len(offsets) != 1:
        raise AssertionError((name, offsets))
    def trace(frame, event, arg):
        if frame.f_code is fn.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == offsets[0]:
                events.append('hit')
                raise Fault()
        return trace
    events.clear()
    sys.settrace(trace)
    try:
        try:
            fn(*args)
            raise AssertionError('injection did not escape')
        except Fault:
            pass
    finally:
        sys.settrace(None)
    if events != expected:
        raise AssertionError((name, events))
";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for level in LEVELS {
            let [native, actual] =
                compile_both(SOURCE, "protected_with_join_pass.py", version, level);
            assert_eq!(actual, native);
            let index = VERSIONS
                .iter()
                .position(|(target, _, _)| *target == version)
                .unwrap();
            assert_runs(
                probe(index).args(["-I", "-S", "-c", TRACE]),
                [&native, &actual],
            );
        }
    }
}

#[test]
fn optimized_assertions_after_shallow_async_for_keep_manager_exit_location() {
    for (case, source) in [
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            pass\n        assert items\n",
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            work(item)\n        assert items\n        assert cm\n",
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            value = item\n        assert items\n",
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            continue\n        assert items\n",
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            assert item\n        assert items\n",
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            pass\n",
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            pass\n        assert items\n        pass\n",
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            pass\n        pass\n        assert items\n",
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            pass\n        assert items\n        work(items)\n",
        "async def f(cm, items):\n    async for item in items:\n        pass\n    assert items\n",
        "async def f(cm, items):\n    async with cm:\n        async for item in items:\n            pass\n        assert items\n",
        "async def f(cm, items):\n    with cm:\n        for item in items:\n            pass\n        assert items\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("shallow_async_for_manager_exit_{case}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn optimized_assertions_after_async_for_preserve_manager_exit_marker() {
    const SOURCE: &str = "async def f(xs,a,b,c):\n    with a:\n        u = 1\n        async for x in xs:\n            pass\n        assert xs\n";
    const TRACE: &str = r"
import asyncio, dis, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
instructions = list(dis.get_instructions(f))
end = next(i for i, instruction in enumerate(instructions)
           if instruction.opname == 'END_ASYNC_FOR')
assert instructions[end + 1].opname == 'NOP'
class Fault(Exception):
    pass
class Manager:
    def __enter__(self):
        events.append('enter')
        return self
    def __exit__(self, typ, value, tb):
        events.append(('exit', typ.__name__ if typ else None))
        return False
class Iter:
    def __aiter__(self):
        return self
    async def __anext__(self):
        raise StopAsyncIteration
for target, expected in (
    (instructions[end].offset, ['enter', ('exit', 'Fault'), 'escaped']),
    (instructions[end + 1].offset, ['enter', 'escaped']),
):
    events, hits = [], []
    def trace(frame, event, arg):
        if frame.f_code is f.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == target:
                hits.append(target)
                raise Fault()
        return trace
    sys.settrace(trace)
    try:
        try:
            asyncio.run(f(Iter(), Manager(), None, None))
        except Fault:
            events.append('escaped')
    finally:
        sys.settrace(None)
    assert hits == [target] and events == expected, (hits, events, expected)
";
    for source in [
        SOURCE,
        "async def f(xs,a,b,c):\n    with a:\n        u = 1\n        async for x in xs:\n            work(x)\n        assert xs; assert a\n",
        "async def f(xs,a,b,c):\n    async with a:\n        with b:\n            u = 1\n            async for x in xs:\n                pass\n            assert xs\n",
        "async def f(xs,a,b,c):\n    async with a:\n        u = 1\n        async for x in xs:\n            pass\n        assert xs\n",
        "async def f(xs,a,b,c):\n    with a:\n        u = 1\n        async for x in xs:\n            if x:\n                break\n            work(x)\n        assert xs\n",
        "async def f(xs,a,b,c):\n    with a:\n        u = 1\n        async for x in xs:\n            pass\n        assert xs\n        work(a)\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "async_for_assert.py",
            &LEVELS,
            &[PythonVersion::Py314],
        );
    }
    for level in [OptimizationLevel::One, OptimizationLevel::Two] {
        let [native, actual] =
            compile_both(SOURCE, "async_for_assert.py", PythonVersion::Py314, level);
        assert_eq!(actual, native);
        assert_runs(probe(2).args(["-I", "-S", "-c", TRACE]), [&native, &actual]);
    }
}

#[test]
fn optimized_footer_after_instruction_free_branch_tail() {
    let template = "def f(x, y, z, value):\n    if x:\n        if y:\n            work()\n        TAIL\n    else:\n        other()\n    after()\n    if y:\n        if z:\n            pass\n        assert z\n";
    for tail in [
        "assert value",
        "global marker",
        "pass\n        global marker",
    ] {
        let source = template.replace("TAIL", tail);
        assert_matches_cpython_versions(
            &source,
            "tail_and_footer.py",
            &LEVELS,
            &[PythonVersion::Py313],
        );
    }
}

#[test]
fn optimized_footer_after_dotted_import_in_same_module() {
    for import in [
        "if False:\n    from SercOuter.SercInner.Leaf import item\n",
        "from SercOuter.SercInner import item\n",
        "if False:\n    import SercOuter.SercInner.Leaf\n",
    ] {
        let source = format!(
            "{import}class SercOuter:\n    class SercInner:\n        pass\ndef f(x, y):\n    if x:\n        if y:\n            pass\n        assert y\n"
        );
        assert_matches_cpython_versions(
            &source,
            "dotted_import_footer.py",
            &LEVELS,
            &[PythonVersion::Py313],
        );
    }
}

#[test]
fn instruction_free_branch_tails_preserve_join_locations() {
    let template = "def f(x, y, value):\n    if x:\n        if y:\n            work()\n        TAIL\n    else:\n        other()\n    after()\n";
    for tail in [
        "assert value",
        "global marker",
        "pass\n        global marker",
    ] {
        let source = template.replace("TAIL", tail);
        assert_matches_cpython(&source, "branch_tail.py", &LEVELS);
    }
    // The same join may require an EXTENDED_ARG for a long jump.
    let long_jump = template
        .replace("TAIL", "assert value")
        .replace("        other()\n", &"        other()\n".repeat(90));
    assert_matches_cpython(&long_jump, "branch_tail.py", &LEVELS);

    for source in [
        r"def enclosing():
    cell = None
    def f(x, y, value, manager):
        with manager:
            if x:
                if y:
                    work()
                nonlocal cell
                assert value
            else:
                other()
            after()
    return f
",
        r"async def f(x, y, value):
    try:
        if x:
            if y:
                work()
            assert value
        else:
            other()
        after()
    finally:
        cleanup()
",
        r"def f(x, y, items):
    if x:
        for item in items:
            if y:
                break
            work()
        else:
            other()
        global marker
    else:
        other()
    after()
",
        r"def f(x, y, z):
    if x:
        while y:
            if z:
                break
            work()
        else:
            other()
        global marker
    else:
        other()
    after()
",
    ] {
        assert_matches_cpython(source, "branch_tail.py", &LEVELS);
    }
}

#[test]
fn erased_branch_assertions_preserve_copied_footer_donors() {
    for (case, source) in [
        r"def example(flag, choice, build):
    if flag:
        if choice:
            value = build()
            assert value
        else:
            assert choice
            value = build() if flag else choice
        assert flag
    else:
        return build()
",
        r"def example(flag, choice, build):
    if flag:
        if choice:
            value = build()
            assert value
        else:
            assert choice
            value = build()
        assert flag
    else:
        return build()
",
        r"def example(flag, choice, build):
    if flag:
        if choice:
            value = build()
        else:
            value = build() if flag else choice
        assert flag
    else:
        return build()
",
        r"def example(flag, choice, build):
    if flag:
        if choice:
            value = build()
        else:
            value = build()
        assert flag
    else:
        return build()
",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("copied_binary_footer_{case}.py"), &LEVELS);
    }
}

#[test]
fn nested_leaf_returns_preserve_repeated_scope_footer() {
    for (case, source) in [
        r"def f(a, b, c):
    if a:
        assert a
    if b:
        if c:
            return None
",
        r"def f(a, b, c):
    if a:
        assert a
    if b:
        if c:
            return 1
",
        r"def f(a, b, c):
    if a:
        assert a
    if b:
        if c:
            return a
",
        r"def f(a, b, c):
    if a:
        pass
    if b:
        if c:
            return None
",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("nested_leaf_footer_{case}.py"), &LEVELS);
    }
}

#[test]
fn normal_finalizer_none_marker_reuses_original_local_load_owner() {
    let source = r"async def f(outer, inner):
    async with outer:
        resource = None
        try:
            resource = await setup()
            await prepare(resource)
            async with inner:
                value = None
                pending = await begin()
                try:
                    await work()
                    response = await read()
                    value = consume(response)
                finally:
                    remaining = await finish(pending)
            assert value is not None
            check(remaining, value)
            assert response
        finally:
            if resource is not None:
                try:
                    await cleanup(resource)
                except Error:
                    report()
";
    for (index, source) in [
        source.to_owned(),
        source.replace("            check(remaining, value)\n", ""),
        source.replace("            assert value is not None\n", "").replace("            assert response\n", ""),
        source.replace("                try:\n                    await cleanup(resource)\n                except Error:\n                    report()\n", "                await cleanup(resource)\n"),
    ].into_iter().enumerate() {
        assert_matches_cpython(&source, &format!("normal_finalizer_local_load_owner_{index}.py"), &LEVELS);
    }
}

#[test]
fn erased_iterator_else_stays_inside_enclosing_finally() {
    let source = r"def f(items, expected, count):
    try:
        result = prepare()
    except Failure:
        raise
    else:
        if count > 1:
            for item in items:
                value = item.read()
                current = extract(value)
                if expected != checksum(current):
                    report(expected, current)
                else:
                    assert result == value
    finally:
        close()
";
    for (index, source) in [
        source.to_owned(),
        source.replace(
            "                else:\n                    assert result == value\n",
            "                else:\n                    pass\n",
        ),
        source.replace(
            "                    assert result == value\n",
            "                    consume(value)\n",
        ),
        source.replace(
            "                    assert result == value\n",
            "                    assert result == value\n                finish(item)\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("finally_iterator_else_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_conditional_join_precedes_named_awaitable_finalizer() {
    let source = r"async def f(resource, cleanup):
    try:
        await work(resource)
        if resource.expected is not None:
            assert resource.value is not None
            assert resource.expected in resource.value
        else:
            assert resource.value is None
    finally:
        await cleanup
";
    for (index, source) in [
        source.to_owned(),
        source.replace("f(resource, cleanup)", "f(resource)"),
        source.replacen(
            "    try:\n",
            "    def capture():\n        return cleanup\n    try:\n",
            1,
        ),
        source.replace(
            "        await cleanup\n",
            "        if resource:\n            await cleanup\n        await another\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("named_awaitable_finalizer_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_iterator_arms_stay_inside_enclosing_except_and_finally() {
    let source = r"def normalize(groups, enabled):
    for group in groups:
        try:
            for value in group:
                item = prepare(value)
                consume(item)
                if enabled:
                    assert item
                else:
                    assert group
            consume(group)
        except LookupError:
            recover()
        finally:
            cleanup(group)
";
    for (index, source) in [
        source.to_owned(),
        source.replace("        except LookupError:\n            recover()\n", ""),
        source.replace(
            "item = prepare(value)",
            "item = prepare(value) if enabled else fallback(value)",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("protected_erased_iterator_arms_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn managed_coroutine_erased_shared_arm_preserves_copied_loop_exits() {
    let source = r"async def f(items, outer, manager, enabled, extra):
    async with outer:
        with manager:
            for item in items:
                if item.kind:
                    if enabled or extra:
                        assert item.valid
                    else:
                        value = item.value
                        assert value
                else:
                    consume(item)
";
    for source in [
        source.to_owned(),
        source.replace("async with outer:", "with outer:"),
        source.replace("with manager:", "with manager, other:"),
        source.replace("enabled or extra", "enabled or extra or item.third"),
    ] {
        assert_matches_cpython(&source, "managed_coroutine_shared_loop_exit.py", &LEVELS);
    }
}

#[test]
fn coroutine_erased_join_before_nested_iterator() {
    let source = r"async def f(items, later):
    with manager():
        await prepare()
        for item in items:
            if item.ready:
                assert item.valid
            else:
                assert item.other
            for entry in later:
                consume(entry)
";
    for source in [
        source.to_owned(),
        source.replace("in later:", "in source.items:"),
        source.replace("in later:", "in entries():"),
        source.replace("with manager():", "with manager(), manager():"),
    ] {
        assert_matches_cpython(&source, "coroutine_iterator_successor.py", &LEVELS);
    }
}

#[test]
fn protected_iterator_tail_accepts_completed_conditional_work() {
    let source = r"async def f(items, outer, manager, config, key, legacy):
    async with outer:
        with manager:
            for item in items:
                if item.kind:
                    consume(item)
                else:
                    value = item.value
                    if config is not None and key in config:
                        start = 1 if item.first else 6
                        if legacy:
                            assert value == start
                        else:
                            assert value != start
                    else:
                        assert value
";
    for source in [
        source.to_owned(),
        source.replace("1 if item.first else 6", "first if item.first else second"),
        source.replace(
            "1 if item.first else 6",
            "left() if item.first else right()",
        ),
        source.replace("if legacy:", "if item.legacy:"),
    ] {
        assert_matches_cpython(&source, "protected_iterator_completed_work.py", &LEVELS);
    }
}

#[test]
fn erased_match_case_threads_through_enclosing_branch() {
    for calls in [2, 30] {
        for pattern in ["A() | B()", "A()"] {
            let mut source = format!(
                "def f(subject, enabled, emit):\n    if enabled:\n        match subject:\n            case {pattern}:\n                assert False\n            case C():\n"
            );
            for index in 0..calls {
                writeln!(source, "                emit({index})").unwrap();
            }
            source.push_str(
                "            case _:\n                emit(subject)\n    else:\n        emit(enabled)\n    emit(subject)\n    emit(enabled)\n    return subject\n",
            );
            assert_matches_cpython(&source, "erased_match_branch.py", &LEVELS);
        }
    }
}

#[test]
fn terminal_nested_erased_if_keeps_finalizer_join() {
    let source = r"async def f(first, second):
    try:
        if first:
            consume(first)
            assert first
        else:
            consume(second)
            assert second
            if second:
                assert first
            else:
                assert second
    finally:
        cleanup()
";
    for (index, source) in [
        source.to_owned(),
        source.replace("async def", "def"),
        source.replace("    finally:", "            assert first\n    finally:"),
        source.replace(
            "            if second:",
            "            if first:\n                consume(first)\n            elif second:",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("terminal_nested_finalizer_join_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn boolean_call_prefix_keeps_terminal_finalizer_join() {
    let source = r#"async def f(first, second, mapping):
    try:
        if first:
            consume(first)
            assert first
        else:
            value = coerce(mapping.get("entry") or {})
            assert value
            if second:
                assert first
            else:
                assert second
    finally:
        cleanup()
"#;
    for (index, source) in [
        source.to_owned(),
        source.replace("async def", "def"),
        source.replace(" or {}", " and {}"),
        source.replace("coerce(mapping", "factory.coerce(value=mapping"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("boolean_call_finalizer_prefix_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn while_break_retains_enclosing_try_else_owner() {
    for scope in ["def", "async def"] {
        for condition in ["True", "flag"] {
            for continuation in [
                "pass",
                "consume(value)",
                "consume(value, mode='value')",
                "obj.consume({'value': value, 'ready': True})",
            ] {
                let source = format!(
                    "{scope} f(value, flag, obj):\n\
                     \x20   try:\n\
                     \x20       while {condition}:\n\
                     \x20           if flag:\n\
                     \x20               break\n\
                     \x20           consume(value)\n\
                     \x20   except Exception:\n\
                     \x20       consume(value)\n\
                     \x20   else:\n\
                     \x20       {continuation}\n\
                     \x20   finish(value)\n"
                );
                assert_matches_cpython(&source, "while_break_try_else.py", &LEVELS);
            }
        }
    }
}

#[test]
fn try_except_end_preserves_original_borrow_boundary() {
    let source = include_str!("fixtures/try_except_borrow_boundary.py");
    for (index, source) in [
        source.to_owned(),
        source.replace("async def", "def").replace("await ", ""),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("try_except_borrow_boundary_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn runtime_loop_enters_iterator_finalizer_314() {
    for guarded in [false, true] {
        for managers in [0, 2] {
            for handler in ["Error", "Error as problem"] {
                let body = if guarded {
                    let loop_body =
                        "while state.ready:\n    if measure() >= limit:\n        break\n    pass\n";
                    format!(
                        "if ready:\n    {}",
                        loop_body.replace('\n', "\n    ").trim_end()
                    )
                } else {
                    "while state.ready:\n    pass\n".to_owned()
                };
                let mut suite = format!(
                    "try:\n    {}\nexcept {handler}:\n    pass\nfinally:\n    for item in (first, second):\n        pass\n",
                    body.replace('\n', "\n    ").trim_end(),
                );
                for manager in 0..managers {
                    suite = format!(
                        "with manager{manager}:\n    {}\n",
                        suite.replace('\n', "\n    ").trim_end()
                    );
                }
                let source = format!(
                    "def sample(ready, state):\n    first = second = None\n    {}\n    return state.value\n",
                    suite.replace('\n', "\n    ").trim_end(),
                );
                assert_matches_cpython_versions(
                    &source,
                    "runtime_loop_iterator_finalizer.py",
                    &LEVELS,
                    &[PythonVersion::Py314],
                );
            }
        }
    }
}

#[test]
fn guarded_handler_recovery_reaches_iterator_finalizer_314() {
    for handler in ["Error", "Error as problem"] {
        for nested in ["pass", "recover()"] {
            for cleanup in ["pass", "release(item)"] {
                for guard in ["first is not None", "state.recover"] {
                    let source = format!(
                        "def sample(state, first, second):\n    try:\n        while state.active:\n            step()\n    except {handler}:\n        if {guard}:\n            try:\n                {nested}\n            except LookupError:\n                pass\n    finally:\n        for item in (first, second):\n            {cleanup}\n",
                    );
                    assert_matches_cpython_versions(
                        &source,
                        "guarded_handler_iterator_finalizer.py",
                        &LEVELS,
                        &[PythonVersion::Py314],
                    );
                }
            }
        }
    }
}

#[test]
fn coroutine_nested_recovery_reaches_finalizer_guard_314() {
    for work in [
        "pass",
        "work()",
        "if state.done:\n    return\nwork()",
        "if state.done:\n    return state.value\nwork()",
        "if state.done:\n    if state.ready:\n        return finish(state)\nwork()",
        "while state.pending:\n    if state.done:\n        return state.value\n    if state.retry:\n        continue\n    await poll(state)\nwork()",
    ] {
        for guard in ["state.ready", "state.ready and accept(problem)"] {
            for named in [false, true] {
                for handler_count in [1, 2] {
                    let first = if named {
                        "FirstError as first"
                    } else {
                        "FirstError"
                    };
                    let mut recovery = format!(
                        "try:\n    if retryable(problem):\n        raise problem\nexcept {first}:\n    pass",
                    );
                    if handler_count == 2 {
                        let second = if named {
                            "SecondError as second"
                        } else {
                            "SecondError"
                        };
                        recovery.push_str("\nexcept ");
                        recovery.push_str(second);
                        recovery.push_str(":\n    pass");
                    }
                    let source = format!(
                        "async def sample(state):\n    try:\n        {}\n    except Error as problem:\n        if {guard}:\n            {}\n    finally:\n        if state.ready:\n            pass\n",
                        work.replace('\n', "\n        "),
                        recovery.replace('\n', "\n            "),
                    );
                    assert_matches_cpython_versions(
                        &source,
                        "coroutine_nested_recovery_finalizer_guard.py",
                        &LEVELS,
                        &[PythonVersion::Py314],
                    );
                }
            }
        }
    }
    for work in ["pass", "work()"] {
        let source = format!(
            "async def sample(state):\n    try:\n        {work}\n    except Error as problem:\n        if state.ready:\n            try:\n                try:\n                    if retryable(problem):\n                        raise problem\n                except FirstError:\n                    pass\n            except SecondError:\n                pass\n    finally:\n        if state.ready:\n            pass\n",
        );
        assert_matches_cpython_versions(
            &source,
            "coroutine_nested_recovery_finalizer_boundary.py",
            &LEVELS,
            &[PythonVersion::Py314],
        );
    }
}

#[test]
fn open_elif_join_reaches_nested_finalizer_314() {
    for function in ["def", "async def"] {
        for (first, second) in [
            ("flag", "other"),
            ("previous is None", "previous != state.key"),
        ] {
            for work in ["pass", "work(state)", "value = 1 if flag else 2"] {
                for value in ["state.kind", "state.kind or 'fallback'"] {
                    for later in [
                        "pass",
                        "if flag:\n    return state",
                        "with manager:\n    if flag:\n        return state",
                        "async with manager:\n    if flag:\n        return state",
                    ] {
                        if function == "def" && later.starts_with("async with") {
                            continue;
                        }
                        // Boolean copied cleanup under a sync manager has a separate
                        // existing exception-table ownership mismatch.
                        if later.starts_with("with") && value != "state.kind" {
                            continue;
                        }
                        let source = format!(
                            "{function} sample(state, previous, flag, other):\n    try:\n        value = flag and prepare()\n        if {first}:\n            {work}\n        elif {second}:\n            {}\n    except Error:\n        pass\n    finally:\n        try:\n            value = {value}\n        except Error:\n            pass\n",
                            later.replace('\n', "\n            "),
                        );
                        assert_matches_cpython_versions(
                            &source,
                            "open_elif_nested_finalizer.py",
                            &LEVELS,
                            &[PythonVersion::Py314],
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn completed_recovery_reaches_nested_awaited_cleanup_guard() {
    for initialized in ["", "    ready = False\n"] {
        for handler in ["Error", "(Error, OtherError)"] {
            for guard in ["flag", "retry(problem)"] {
                for final_work in ["pass", "finish(state)"] {
                    let source = format!(
                        "async def sample(state, flag, other, ready):\n{initialized}    try:\n        operation(state)\n    except {handler} as problem:\n        if flag:\n            try:\n                pass\n            except RecoveryError:\n                pass\n        if {guard}:\n            raise\n    finally:\n        try:\n            if not ready and not other:\n                await cleanup(state)\n        finally:\n            {final_work}\n"
                    );
                    assert_matches_cpython_versions(
                        &source,
                        "nested_awaited_cleanup_guard.py",
                        &LEVELS,
                        &[PythonVersion::Py314],
                    );
                }
            }
        }
    }
}

#[test]
fn runtime_while_reaches_nested_finalizer_entry_314() {
    for function in ["def", "async def"] {
        for guard in ["flag", "flag or other"] {
            for body in ["pass", "if other:\n    break"] {
                for handler in ["pass", "raise", "await recover(state)\nraise"] {
                    if function == "def" && handler.starts_with("await") {
                        continue;
                    }
                    for cleanup in [
                        "pass",
                        "finish(state)",
                        "for item in items:\n    finish(item)",
                    ] {
                        let source = format!(
                            "{function} sample(state, items, flag, other):\n    try:\n        while {guard}:\n            {}\n    except Error:\n        {}\n    finally:\n        try:\n            state.result = work(state, items)\n        finally:\n            {}\n",
                            body.replace('\n', "\n            "),
                            handler.replace('\n', "\n        "),
                            cleanup.replace('\n', "\n            "),
                        );
                        assert_matches_cpython_versions(
                            &source,
                            "while_nested_finalizer_entry.py",
                            &LEVELS,
                            &[PythonVersion::Py314],
                        );
                    }
                }
            }
        }
    }
}
