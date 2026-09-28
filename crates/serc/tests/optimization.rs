use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

use serc::{
    CompileOptions, OptimizationLevel, PythonVersion, compile, compile_bytes_with_options,
    compile_bytes_with_path_and_options, compile_with_options,
};

const VERSIONS: [(PythonVersion, &str, &str); 4] = [
    (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
    (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
    (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
    (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
];
const LEVELS: [OptimizationLevel; 3] = [
    OptimizationLevel::Zero,
    OptimizationLevel::One,
    OptimizationLevel::Two,
];

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
fn chained_condition_loop_exit_preserves_iterator_location() {
    for source in [
        "def f(a, b, items):\n    assert a\n    if check(a):\n        assert b\n    if a < b < items:\n        assert a\n        for item in items:\n            assert item\n",
        "def f(a, b, items):\n    if a < b < items:\n        for item in items:\n            consume(item)\n",
        "def f(a, b, items):\n    if a < b:\n        for item in items:\n            consume(item)\n",
        "def f(a, b, items):\n    if a < b < items:\n        for item in items:\n            consume(item)\n        after()\n",
        "def f(a, b, items):\n    if a < b < items:\n        for item in items:\n            consume(item)\n    after()\n",
    ] {
        assert_matches_cpython(source, "chained_condition_loop_exit.py", &LEVELS);
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
fn finally_declarations_preserve_exceptional_exit_locations() {
    assert_matches_cpython(
        "def f(manager, flag):\n    with manager:\n        try:\n            work()\n        finally:\n            if flag:\n                cleanup()\n            global marker\n",
        "finally_global_declaration.py",
        &LEVELS,
    );
    assert_matches_cpython(
        "def outer():\n    marker = None\n    def f(manager, flag):\n        with manager:\n            try:\n                work()\n            finally:\n                if flag:\n                    cleanup()\n                nonlocal marker\n    return f\n",
        "finally_nonlocal_declaration.py",
        &LEVELS,
    );
}

#[test]
fn conditional_assignment_preserves_original_statement_joins() {
    for source in [
        r"def example(flag, choice, build):
    if flag:
        if choice:
            value = build() if flag else choice
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
            value = build() if flag else choice
            assert value
        else:
            assert choice
            value = build()
        pass
    else:
        return build()
",
    ] {
        assert_matches_cpython(source, "conditional_assignment_statement_joins.py", &LEVELS);
    }
}

#[test]
fn folded_true_value_selector_preserves_empty_borrow_entry() {
    for source in [
        "def probe(marker, low, high, bound, choice, flag, left, right, obj, make):\n    if marker is not None:\n        if low >= bound:\n            scratch = left\n        else:\n            scratch = right\n        if low != high:\n            scratch = right\n        value = obj.payload if __debug__ else right\n        if flag:\n            result = None\n        else:\n            result = make()\n    else:\n        assert marker\n    return marker\n",
        "def probe(marker, low, high, bound, choice, flag, left, right, obj, make):\n    if marker is not None:\n        if low >= bound:\n            scratch = left\n        else:\n            scratch = right\n        if low != high:\n            scratch = right\n        value = obj.payload if True else right\n        if flag:\n            result = None\n        else:\n            result = make()\n    else:\n        assert marker\n    return marker\n",
        "def probe(marker, low, high, bound, choice, flag, left, right, obj, make):\n    if marker is not None:\n        if low >= bound:\n            scratch = left\n        else:\n            scratch = right\n        if low != high:\n            scratch = right\n        value = obj.child.payload if __debug__ else right\n        if flag:\n            result = None\n        else:\n            result = make()\n    else:\n        assert marker\n    return marker\n",
    ] {
        assert_matches_cpython(source, "folded_true_value_selector.py", &LEVELS);
    }
}

#[test]
fn elided_suffix_preserves_populated_nested_if_join() {
    let source = "def f(outer, inner, value, obj):\n    if outer is None:\n        if inner:\n            result = 1\n        else:\n            result = value + 1\n        assert inner\n    else:\n        assert outer\n    after = obj.attribute\n    return after\n";
    assert_matches_cpython(source, "nested_if_suffix.py", &LEVELS);
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
fn elided_else_after_completed_work_preserves_legacy_footer_location() {
    for source in [
        "def f(flag, enabled, count, inner, obj):\n    if not flag:\n        assert flag\n        assert count\n    if enabled and count > 0:\n        assert enabled\n        value = obj.make()\n        use(value)\n        if inner:\n            use(value)\n            use(value)\n        value = 1.0\n        assert value\n        other = 2.0\n        use(other)\n    else:\n        assert count\n",
        "def f(flag, enabled, count, inner, obj):\n    if enabled and count > 0:\n        assert enabled\n        value = obj.make()\n        use(value)\n        if inner:\n            use(value)\n            use(value)\n        value = 1.0\n        assert value\n        other = 2.0\n        use(other)\n    else:\n        assert count\n",
        "def f(flag, enabled, count, inner, obj):\n    if not flag:\n        assert flag\n        assert count\n    if enabled:\n        assert enabled\n        value = obj.make()\n        use(value)\n        if inner:\n            use(value)\n            use(value)\n        value = 1.0\n        assert value\n        other = 2.0\n        use(other)\n    else:\n        assert count\n",
        "def f(flag, enabled, count, inner, obj):\n    if not flag:\n        assert flag\n        assert count\n    if enabled and count > 0:\n        assert enabled\n        value = obj.make()\n        use(value)\n        value = 1.0\n        assert value\n        other = 2.0\n        use(other)\n    else:\n        assert count\n",
        "def f(flag, enabled, count, inner, obj):\n    if not flag:\n        assert flag\n        assert count\n    if enabled and count > 0:\n        value = obj.make()\n        use(value)\n        if inner:\n            use(value)\n            use(value)\n        value = 1.0\n        other = 2.0\n        use(other)\n    else:\n        assert count\n",
        "def f(flag, enabled, count, inner, obj):\n    if not flag:\n        assert flag\n        assert count\n    if enabled and count > 0:\n        value = obj.make()\n        use(value)\n        value = 1.0\n        other = 2.0\n        use(other)\n    else:\n        assert count\n",
    ] {
        assert_matches_cpython(source, "completed_work_else.py", &LEVELS);
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
fn erased_suffix_after_breaking_for_preserves_branch_join() {
    for source in [
        "def example(items, candidates, size, mode, obj, ready, alternate, use):\n    if size > 0:\n        before = prepare()\n        if ready:\n            use(before)\n        middle = prepare()\n        if alternate:\n            use(middle)\n        if mode == 0:\n            total = 0\n            for item in items:\n                if item == mode:\n                    total = 0\n                    total += item\n            assert total\n            picked = 0\n            for candidate in candidates:\n                assert candidate\n                if candidate == mode:\n                    picked = candidate.value\n                    picked += total\n                    if picked == mode:\n                        break\n            assert picked\n        else:\n            assert size\n        result = finish(obj)\n        use(result)\n",
        "def example(items, candidates, size, mode, obj, ready, alternate, use):\n    if size > 0:\n        before = prepare()\n        if ready:\n            use(before)\n        middle = prepare()\n        if alternate:\n            use(middle)\n        if mode == 0:\n            total = 0\n            assert total\n            picked = 0\n            for candidate in candidates:\n                assert candidate\n                if candidate == mode:\n                    picked = candidate.value\n                    picked += total\n                    if picked == mode:\n                        break\n            assert picked\n        else:\n            assert size\n        result = finish(obj)\n        use(result)\n",
        "def example(items, candidates, size, mode, obj, ready, alternate, use):\n    if size > 0:\n        before = prepare()\n        if ready:\n            use(before)\n        middle = prepare()\n        if alternate:\n            use(middle)\n        if mode == 0:\n            total = 0\n            for item in items:\n                if item == mode:\n                    total = 0\n                    total += item\n            assert total\n            picked = 0\n            for candidate in candidates:\n                assert candidate\n                if candidate == mode:\n                    picked = candidate.value\n                    picked += total\n            assert picked\n        else:\n            assert size\n        result = finish(obj)\n        use(result)\n",
        "def example(items, candidates, size, mode, obj, ready, alternate, use):\n    if size > 0:\n        before = prepare()\n        if ready:\n            use(before)\n        middle = prepare()\n        if alternate:\n            use(middle)\n        if mode == 0:\n            total = 0\n            for item in items:\n                if item == mode:\n                    total = 0\n                    total += item\n            assert total\n            picked = 0\n            for candidate in candidates:\n                assert candidate\n                if candidate == mode:\n                    picked = candidate.value\n                    picked += total\n                    if picked == mode:\n                        break\n            assert picked\n        else:\n            pass\n        result = finish(obj)\n        use(result)\n",
        "def example(items, candidates, size, mode, obj, ready, alternate, use):\n    if size > 0:\n        before = prepare()\n        if ready:\n            use(before)\n        middle = prepare()\n        if alternate:\n            use(middle)\n        if mode == 0:\n            total = 0\n            picked = obj.value\n            picked += total\n            assert picked\n        else:\n            assert size\n        result = finish(obj)\n        use(result)\n",
        "def example(items, candidates, size, mode, obj, ready, alternate, use):\n    if size > 0:\n        before = prepare()\n        if ready:\n            use(before)\n        middle = prepare()\n        if alternate:\n            use(middle)\n        if mode == 0:\n            total = 0\n            for item in items:\n                if item == mode:\n                    total = 0\n                    total += item\n            assert total\n            picked = 0\n            for candidate in candidates:\n                assert candidate\n                if candidate == mode:\n                    picked = candidate.value\n                    picked += total\n                    if picked == mode:\n                        break\n            assert picked\n        else:\n            assert size\n",
    ] {
        assert_matches_cpython(source, "breaking_for_branch_join.py", &LEVELS);
    }
}

#[test]
fn direct_call_and_continue_preserve_legacy_for_footer_location() {
    for source in [
        "def f(flag, active, items):\n    if flag is not None:\n        assert flag\n    if active is not None:\n        assert active\n        for item in items:\n            if item is None:\n                continue\n            consume(item)\n",
        "def f(flag, active, items):\n    if active is not None:\n        assert active\n        for item in items:\n            if item is None:\n                continue\n            consume(item)\n",
        "def f(flag, active, items):\n    if flag is not None:\n        assert flag\n    if active is not None:\n        assert active\n        for item in items:\n            consume(item)\n",
        "def f(flag, active, items):\n    if flag is not None:\n        assert flag\n    if active is not None:\n        assert active\n        for item in items:\n            item.consume()\n",
        "def f(flag, active, items):\n    if flag is not None:\n        assert flag\n    if active is not None:\n        assert active\n        for item in items:\n            if item is None:\n                continue\n            item.consume()\n",
        "def f(flag, active, items):\n    if flag is not None:\n        consume(flag)\n    if active is not None:\n        assert active\n        for item in items:\n            if item is None:\n                continue\n            consume(item)\n",
    ] {
        assert_matches_cpython(source, "continue_call_for.py", &LEVELS);
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
fn assigned_conditional_prefix_preserves_populated_nested_join() {
    for successor in ["    after = obj\n", "    after = obj.after\n"] {
        let source = format!(
            "def f(obj, a, b, c, d, work):\n    if obj is not None:\n        if a:\n            first = work()\n        else:\n            first = work()\n        if b:\n            second = work()\n        chosen = c if d else a\n        if c:\n            result = None\n        else:\n            result = work()\n    else:\n        assert obj\n{successor}    return after\n"
        );
        assert_matches_cpython(&source, "assigned_conditional_prefix_join.py", &LEVELS);
    }
}

#[test]
fn calls_before_return_preserve_erased_loop_tail() {
    let source = "def f(items, blocked, enabled, work, obj):\n    for item in items:\n        if item not in blocked:\n            if enabled:\n                work()\n                return work()\n            assert item\n    return work()\n";
    assert_matches_cpython(source, "calls_before_return_loop_tail.py", &LEVELS);
}

#[test]
fn completed_prefix_preserves_populated_nested_join() {
    let source = "def f(obj, a, b, c, d, work):\n    if obj is not None:\n        assert obj.ready\n        before = obj\n        if a:\n            first = work()\n        else:\n            first = work()\n        between = obj.ready\n        assert a\n        if b:\n            second = work()\n        chosen = c if d else a\n        assert b\n        after_choice = obj.ready\n        if c:\n            result = None\n        else:\n            result = work()\n    else:\n        assert obj\n    after = obj.after\n    return after\n";
    assert_matches_cpython(source, "completed_prefix_join.py", &LEVELS);
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
fn completed_generator_call_preserves_nested_branch_join() {
    for source in [
        "def f(value, obj, items):\n    if value is None:\n        if obj.flag:\n            values = tuple(item.value for item in items)\n            value = obj.make(values)\n        else:\n            obj.check()\n            value = obj.build()\n    else:\n        assert value == obj.other\n    value = obj.finish(value)\n    return obj.wrap(value)\n",
        "def f(value, obj, items):\n    if value is None:\n        if obj.flag:\n            values = all(item.value for item in items)\n            value = obj.make(values)\n        else:\n            obj.check()\n            value = obj.build()\n    else:\n        assert value == obj.other\n    value = obj.finish(value)\n    return obj.wrap(value)\n",
        "def f(value, obj, items):\n    if value is None:\n        if obj.flag:\n            values = any(item.value for item in items)\n            value = obj.make(values)\n        else:\n            obj.check()\n            value = obj.build()\n    else:\n        assert value == obj.other\n    value = obj.finish(value)\n    return obj.wrap(value)\n",
        "def f(value, obj, items):\n    if value is None:\n        if obj.flag:\n            value = obj.make()\n        else:\n            values = tuple(item.value for item in items)\n            value = obj.make(values)\n    else:\n        assert value == obj.other\n    value = obj.finish(value)\n    return obj.wrap(value)\n",
        "def f(value, obj, items):\n    if value is None:\n        if obj.flag:\n            values = tuple(item.value for item in items)\n            value = obj.make(values)\n        else:\n            values = tuple(item.value for item in items)\n            value = obj.make(values)\n    else:\n        assert value == obj.other\n    value = obj.finish(value)\n    return obj.wrap(value)\n",
        "def f(value, obj, items):\n    if value is None:\n        if obj.flag:\n            value = obj.make()\n        else:\n            obj.check()\n            value = obj.build()\n    else:\n        assert value == obj.other\n    value = obj.finish(value)\n    return obj.wrap(value)\n",
        "def f(value, obj, items):\n    if value is None:\n        if obj.flag:\n            values = tuple(item.value for item in items)\n            value = obj.make(values)\n        else:\n            obj.check()\n            value = obj.build()\n    else:\n        obj.check(value)\n    value = obj.finish(value)\n    return obj.wrap(value)\n",
        "def f(value, obj, items):\n    if value is None:\n        if obj.flag:\n            values = tuple(item.value for item in items)\n            value = obj.make(values)\n        else:\n            obj.check()\n            value = obj.build()\n        obj.after()\n    else:\n        assert value == obj.other\n    value = obj.finish(value)\n    return obj.wrap(value)\n",
    ] {
        assert_matches_cpython(source, "completed_generator_call_join.py", &LEVELS);
    }
    for source in [
        "def f(value, obj, items):\n    if value is None:\n        if obj.flag:\n            values = list(item.value for item in items)\n            value = obj.make(values)\n        else:\n            obj.check()\n            value = obj.build()\n    else:\n        assert value == obj.other\n    value = obj.finish(value)\n    return obj.wrap(value)\n",
        "def f(value, obj, items):\n    if value is None:\n        if obj.flag:\n            values = set(item.value for item in items)\n            value = obj.make(values)\n        else:\n            obj.check()\n            value = obj.build()\n    else:\n        assert value == obj.other\n    value = obj.finish(value)\n    return obj.wrap(value)\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "completed_collection_call_join.py",
            &LEVELS,
            &[PythonVersion::Py315],
        );
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
fn completed_branch_work_preserves_original_call_footer() {
    for source in [
        "def f(a, b, c, obj, items):\n    if a:\n        assert a\n    if b:\n        if c:\n            obj.work()\n        obj.finish()\n",
        "def f(a, b, c, obj, items):\n    if a:\n        assert a\n    if b:\n        with obj:\n            obj.work()\n        obj.finish()\n",
        "def f(a, b, c, obj, items):\n    if a:\n        assert a\n    if b:\n        with obj:\n            with obj.other:\n                obj.work()\n        obj.finish()\n",
        "def f(a, b, c, obj, items):\n    if a:\n        assert a\n    if b:\n        for item in items:\n            if item:\n                continue\n            obj.work(item)\n        obj.finish()\n",
        "def f(a, b, c, obj, items):\n    if a:\n        assert a\n    if b:\n        obj.work([item for item in items])\n        obj.finish()\n",
        "def f(a, b, c, obj, items):\n    if a:\n        assert a\n    if b:\n        with obj:\n            with obj.other:\n                for item in items:\n                    if item:\n                        continue\n                    obj.work(item)\n        obj.work([item for item in items])\n        obj.finish()\n",
        "def f(a, b, c, obj, items):\n    if b:\n        with obj:\n            obj.work()\n        obj.finish()\n",
        "def f(a, b, c, obj, items):\n    if a:\n        pass\n    if b:\n        with obj:\n            obj.work()\n        obj.finish()\n",
        "def f(a, b, c, obj, items):\n    if a:\n        assert a\n    if b:\n        with obj:\n            obj.work()\n        obj.finish()\n    obj.after()\n",
        "def f(a, b, c, obj, items):\n    with obj:\n        if a:\n            assert a\n        if b:\n            if c:\n                obj.work()\n            obj.finish()\n",
    ] {
        assert_matches_cpython(source, "completed_branch_call_footer.py", &LEVELS);
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
fn live_for_body_preserves_original_footer_location() {
    for source in [
        "def f(obj, items):\n    if obj.ready:\n        assert obj.ready\n    if obj.active and obj.target is not None:\n        for item in items:\n            item.action()\n",
        "def f(obj, items):\n    if obj.ready:\n        assert obj.ready\n    if obj.active and obj.target is not None:\n        for item in items:\n            assert item\n",
        "def f(obj, items):\n    if obj.ready:\n        assert obj.ready\n    if obj.active:\n        for item in items:\n            item.action()\n",
        "def f(obj, items):\n    if obj.active and obj.target is not None:\n        for item in items:\n            item.action()\n",
    ] {
        assert_matches_cpython(source, "live_for_footer.py", &LEVELS);
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
fn nested_erased_branch_keeps_original_scope_exit() {
    for source in [
        "def f(obj, active, checked):\n    if obj.ready:\n        assert obj.ready\n    prepared = obj.prepare()\n    if obj.active:\n        if obj.check():\n            assert obj.valid\n        else:\n            assert obj.other\n    else:\n        assert obj.active\n",
        "def f(obj, active, checked):\n    if obj.ready:\n        assert obj.ready\n    prepared = obj.prepare()\n    if active:\n        if obj.check():\n            assert obj.valid\n        else:\n            assert obj.other\n    else:\n        assert obj.active\n",
        "def f(obj, active, checked):\n    if obj.ready:\n        assert obj.ready\n    prepared = obj.prepare()\n    if obj.active:\n        if obj.checked:\n            assert obj.valid\n        else:\n            assert obj.other\n    else:\n        assert obj.active\n",
        "def f(obj, active, checked):\n    if obj.ready:\n        assert obj.ready\n    prepared = obj.prepare()\n    if obj.active:\n        if obj.check():\n            assert obj.valid\n        else:\n            assert obj.other\n",
        "def f(obj, active, checked):\n    prepared = obj.prepare()\n    if obj.active:\n        if obj.check():\n            assert obj.valid\n        else:\n            assert obj.other\n    else:\n        assert obj.active\n",
    ] {
        assert_matches_cpython(source, "nested_erased_footer.py", &LEVELS);
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
fn erased_assertion_after_terminating_guard_preserves_footer_location() {
    for source in [
        "def f(value, items, obj, flag):\n    if value is None:\n        return value\n    else:\n        if items is not None:\n            for item in items:\n                if item in value:\n                    return value\n            raise ValueError(value)\n        assert False\n",
        "def f(value, items, obj, flag):\n    if value is None:\n        return value\n    else:\n        if items is not None:\n            for item in items:\n                if item in value:\n                    return value\n            return value\n        assert False\n",
        "def f(value, items, obj, flag):\n    if value is None:\n        return value\n    else:\n        if items is not None:\n            obj.work()\n            raise ValueError(value)\n        assert False\n",
        "def f(value, items, obj, flag):\n    if value is None:\n        return value\n    else:\n        if items is not None:\n            raise ValueError(value)\n        assert False\n",
        "def f(value, items, obj, flag):\n    if value is None:\n        return value\n    else:\n        if items is not None:\n            if flag:\n                obj.work()\n            raise ValueError(value)\n        assert False\n",
        "def f(value, items, obj, flag):\n    if value is None:\n        return value\n    else:\n        if items is not None:\n            try:\n                obj.work()\n            except ValueError:\n                obj.recover()\n            raise ValueError(value)\n        assert False\n",
        "def f(value, items, obj, flag):\n    if value is None:\n        return value\n    else:\n        if items is not None:\n            with obj:\n                obj.work()\n            raise ValueError(value)\n        assert False\n",
        "def f(value, items, obj, flag):\n    if value is None:\n        return value\n    else:\n        if items is not None:\n            match flag:\n                case 0:\n                    obj.work()\n                case _:\n                    obj.other()\n            raise ValueError(value)\n        assert False\n",
        "def f(value, items, obj, flag):\n    if value is None:\n        return value\n    else:\n        if items is not None:\n            for item in items:\n                if item in value:\n                    return value\n            raise ValueError(value)\n",
        "def f(value, items, obj, flag):\n    if value is None:\n        return value\n    else:\n        if items is not None:\n            for item in items:\n                if item in value:\n                    return value\n            raise ValueError(value)\n    assert False\n",
        "def f(value, items, obj, flag):\n    if value is None:\n        return value\n    else:\n        if items is not None:\n            for item in items:\n                if item in value:\n                    return value\n            raise ValueError(value)\n        pass\n",
        "def f(value, items, obj, flag):\n    if value is None:\n        return value\n    else:\n        if items is not None:\n            for item in items:\n                if item in value:\n                    return value\n            raise ValueError(value)\n        obj.after()\n        assert False\n",
        "def f(value, items, obj, flag):\n    if value is None:\n        return value\n    else:\n        if items is not None or flag:\n            for item in items:\n                if item in value:\n                    return value\n            raise ValueError(value)\n        assert False\n",
        "def f(value, items, obj, flag):\n    if value is None:\n        return value\n    else:\n        if items is not None:\n            for item in items:\n                if item in value:\n                    return value\n            raise ValueError(value)\n        assert False\n    yield value\n",
    ] {
        assert_matches_cpython(source, "erased_assertion_terminating_guard.py", &LEVELS);
    }
}

#[test]
fn erased_handler_assertions_preserve_guard_cleanup_location() {
    for source in [
        "def f(flag, other, obj):\n    try:\n        yield\n    except Error as exc:\n        if flag:\n            raise exc\n        assert flag\n        assert other\n        assert obj.ready\n    else:\n        if not flag:\n            assert False\n",
        "def f(flag, other, obj):\n    try:\n        yield\n    except Error as exc:\n        if flag:\n            raise\n        assert flag\n        assert other\n        assert obj.ready\n    else:\n        if not flag:\n            assert False\n",
        "def f(flag, other, obj):\n    try:\n        yield\n    except Error:\n        if flag:\n            raise Error\n        assert flag\n        assert other\n        assert obj.ready\n    else:\n        if not flag:\n            assert False\n",
        "def f(flag, other, obj):\n    try:\n        obj.work()\n    except Error as exc:\n        if flag:\n            raise exc\n        assert flag\n        assert other\n        assert obj.ready\n    else:\n        if not flag:\n            assert False\n",
        "async def f(flag, other, obj):\n    try:\n        await obj.work()\n    except Error as exc:\n        if flag:\n            raise exc\n        assert flag\n        assert other\n        assert obj.ready\n    else:\n        if not flag:\n            assert False\n",
        "async def f(flag, other, obj):\n    try:\n        yield\n    except Error as exc:\n        if flag:\n            raise exc\n        assert flag\n        assert other\n        assert obj.ready\n    else:\n        if not flag:\n            assert False\n",
        "def f(flag, other, obj):\n    try:\n        yield\n    except Error as exc:\n        if flag:\n            raise exc\n    else:\n        if not flag:\n            assert False\n",
        "def f(flag, other, obj):\n    try:\n        yield\n    except Error as exc:\n        if flag:\n            raise exc\n        pass\n    else:\n        if not flag:\n            assert False\n",
        "def f(flag, other, obj):\n    try:\n        yield\n    except Error as exc:\n        if flag:\n            raise exc\n        assert flag\n        obj.after()\n    else:\n        if not flag:\n            assert False\n",
        "def f(flag, other, obj):\n    try:\n        yield\n    except Error as exc:\n        if flag and other:\n            raise exc\n        assert flag\n        assert other\n        assert obj.ready\n    else:\n        if not flag:\n            assert False\n",
        "def f(flag, other, obj):\n    try:\n        yield\n    except Error as exc:\n        if flag or other:\n            raise exc\n        assert flag\n        assert other\n        assert obj.ready\n    else:\n        if not flag:\n            assert False\n",
        "def f(flag, other, obj):\n    try:\n        yield\n    except Error as exc:\n        if flag:\n            raise exc\n        assert flag\n        assert other\n        assert obj.ready\n",
        "def f(flag, other, obj):\n    try:\n        yield\n    except obj.Error as exc:\n        if flag:\n            raise exc\n        assert flag\n        assert other\n        assert obj.ready\n    else:\n        if not flag:\n            assert False\n",
        "def f(flag, other, obj):\n    for item in obj.items:\n        try:\n            yield\n        except Error as exc:\n            if flag:\n                raise exc\n            assert flag\n            assert other\n            assert obj.ready\n        else:\n            if not flag:\n                assert False\n",
    ] {
        assert_matches_cpython(source, "erased_handler_assertion_cleanup.py", &LEVELS);
    }
    assert_matches_cpython(
        "def f(flag, other, obj):\n    with obj:\n        try:\n            yield\n        except Error as exc:\n            if flag:\n                raise exc\n            assert flag\n            assert other\n            assert obj.ready\n        else:\n            if not flag:\n                assert False\n",
        "protected_erased_handler_assertion_cleanup.py",
        &[OptimizationLevel::One, OptimizationLevel::Two],
    );
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
fn terminal_else_preserves_literal_location_after_manager() {
    assert_matches_cpython(
        "if flag:
    pass
else:
    with manager:
        consume()
    0
",
        "terminal_else_literal.py",
        &LEVELS,
    );
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
fn elided_assertion_after_while_preserves_finally_borrow_boundary() {
    assert_matches_cpython(
        r"def f(obj, count):
    try:
        while condition() and count < 3:
            action()
        assert obj
    except ValueError:
        recover()
    finally:
        obj.close()
",
        "while_finally_borrow.py",
        &LEVELS,
    );
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
                let native = oracle(
                    source.as_bytes(),
                    "return_finally.py",
                    python_version,
                    optimization_level,
                );
                assert!(
                    native.status.success(),
                    "{}",
                    String::from_utf8_lossy(&native.stderr)
                );
                let actual = compile_with_options(
                    source,
                    "return_finally.py",
                    &CompileOptions {
                        python_version,
                        optimization_level,
                        ..CompileOptions::default()
                    },
                )
                .unwrap()
                .marshal();
                assert_eq!(
                    actual, native.stdout,
                    "{python_version}, {optimization_level:?}"
                );
                let mut outputs = Vec::new();
                for payload in [&native.stdout, &actual] {
                    let result = run_with_input(
                        Command::new(ORACLES[index].get().unwrap())
                            .args(["-I", "-S", "-B", "-c", TRACE])
                            .stdout(Stdio::piped()),
                        payload,
                    );
                    assert!(
                        result.status.success(),
                        "{python_version}, {optimization_level:?}: status {}, {}",
                        result.status,
                        String::from_utf8_lossy(&result.stderr)
                    );
                    outputs.push(result.stdout);
                }
                assert_eq!(
                    outputs[0], outputs[1],
                    "{python_version}, {optimization_level:?}"
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
                let native = oracle(
                    source.as_bytes(),
                    "return_older_handler.py",
                    python_version,
                    optimization_level,
                );
                assert!(
                    native.status.success(),
                    "{}",
                    String::from_utf8_lossy(&native.stderr)
                );
                let actual = compile_with_options(
                    source,
                    "return_older_handler.py",
                    &CompileOptions {
                        python_version,
                        optimization_level,
                        ..CompileOptions::default()
                    },
                )
                .unwrap()
                .marshal();
                assert_eq!(
                    actual, native.stdout,
                    "{python_version}, {optimization_level:?}"
                );
                let mut outputs = Vec::new();
                for payload in [&native.stdout, &actual] {
                    let result = run_with_input(
                        Command::new(ORACLES[index].get().unwrap())
                            .args(["-I", "-S", "-B", "-c", trace])
                            .stdout(Stdio::piped()),
                        payload,
                    );
                    assert!(
                        result.status.success(),
                        "{python_version}, {optimization_level:?}: status {}, {}",
                        result.status,
                        String::from_utf8_lossy(&result.stderr)
                    );
                    outputs.push(result.stdout);
                }
                assert_eq!(
                    outputs[0], outputs[1],
                    "{python_version}, {optimization_level:?}"
                );
            }
        }
    }
}

#[test]
fn optimized_terminal_guards_locate_fallthrough_returns() {
    for source in [
        "def f(x):\n    if x == 1:\n        assert x\n",
        "def f(x):\n    if not x:\n        assert x\n    else:\n        assert not x\n",
        "def f(x):\n    if x:\n        assert x\n    else:\n        pass\n",
        "if x == 1:\n    assert x\n",
        "def f(x):\n    if x == 1:\n        assert x\n    return None\n",
        "def f(x):\n    if x == 1:\n        pass\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/terminal_guard_fallthrough.py",
            &LEVELS,
            &[PythonVersion::Py314],
        );
    }
}

#[test]
fn optimized_nested_guards_locate_each_fallthrough_return_on_313() {
    for source in [
        "def f(x, y):\n    if x:\n        if y:\n            pass\n        assert y\n",
        "def f(x, y):\n    if x:\n        if y:\n            work()\n        assert y\n    else:\n        fallback()\n",
        "def f(x, y):\n    if x:\n        if y:\n            try:\n                work()\n            finally:\n                cleanup()\n        assert y\n    else:\n        raise ValueError()\n",
        "def f(x, y):\n    if x and y:\n        if x:\n            pass\n        assert y\n    elif y:\n        fallback()\n    else:\n        return 1\n",
        "def f(x, y):\n    if x:\n        if y:\n            pass\n",
        "def f(manager, x, y):\n    try:\n        with manager:\n            pass\n            work()\n    finally:\n        cleanup()\n    if x:\n        if y:\n            pass\n        assert y\n",
        "def f(manager, x, y):\n    if x:\n        try:\n            with manager:\n                pass\n                work()\n        except Exception:\n            recover()\n        if y:\n            pass\n        assert y\n",
        "def f(manager, x, y):\n    try:\n        with manager:\n            pass\n            assert x\n            work()\n    finally:\n        cleanup()\n    if x:\n        if y:\n            pass\n        assert y\n",
        "if False:\n    from package import *\ndef f(x, y):\n    if x:\n        if y:\n            pass\n        assert y\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/nested_terminal_guard.py",
            &LEVELS,
            &[PythonVersion::Py313],
        );
    }
}

#[test]
fn optimized_footer_after_protected_loop_backedge() {
    for source in [
        "def f(xs, work, recover, x, y):\n    try:\n        work()\n    except ValueError:\n        recover()\n    for item in xs:\n        if item:\n            assert False\n        if item:\n            assert False\n    if x:\n        if y:\n            pass\n        assert y\n",
        "def f(xs, work, recover, x, y):\n    try:\n        try:\n            work()\n        except ValueError:\n            recover()\n    except Exception:\n        recover()\n    for item in xs:\n        if not item:\n            assert False\n        if not item:\n            assert False\n    if x:\n        if y:\n            work()\n        assert y\n    else:\n        recover()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/protected_loop_footer.py",
            &LEVELS,
            &[PythonVersion::Py313],
        );
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
        let native = oracle(
            SOURCE.as_bytes(),
            "manager_footer.py",
            PythonVersion::Py313,
            optimization_level,
        );
        assert!(
            native.status.success(),
            "{}",
            String::from_utf8_lossy(&native.stderr)
        );
        let actual = compile_with_options(
            SOURCE,
            "manager_footer.py",
            &CompileOptions {
                python_version: PythonVersion::Py313,
                optimization_level,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .marshal();
        assert_eq!(actual, native.stdout, "{optimization_level:?}");
        for code in [&native.stdout, &actual] {
            let result = run_with_input(
                Command::new(ORACLES[1].get().unwrap())
                    .args(["-I", "-S", "-B", "-c", TRACE])
                    .stdout(Stdio::piped()),
                code,
            );
            assert!(
                result.status.success(),
                "{optimization_level:?}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
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
        let native = oracle(
            SOURCE.as_bytes(),
            "manager_footer.py",
            PythonVersion::Py313,
            optimization_level,
        );
        assert!(
            native.status.success(),
            "{}",
            String::from_utf8_lossy(&native.stderr)
        );
        let actual = compile_with_options(
            SOURCE,
            "manager_footer.py",
            &CompileOptions {
                python_version: PythonVersion::Py313,
                optimization_level,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .marshal();
        assert_eq!(actual, native.stdout, "{optimization_level:?}");
        for code in [&native.stdout, &actual] {
            let result = run_with_input(
                Command::new(ORACLES[1].get().unwrap())
                    .args(["-I", "-S", "-B", "-c", TRACE])
                    .stdout(Stdio::piped()),
                code,
            );
            assert!(
                result.status.success(),
                "{optimization_level:?}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
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
            let native = oracle(
                SOURCE.as_bytes(),
                "manager_finally_footer.py",
                python_version,
                optimization_level,
            );
            assert!(
                native.status.success(),
                "{}",
                String::from_utf8_lossy(&native.stderr)
            );
            let actual = compile_with_options(
                SOURCE,
                "manager_finally_footer.py",
                &CompileOptions {
                    python_version,
                    optimization_level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            assert_eq!(
                actual, native.stdout,
                "{python_version}, {optimization_level:?}"
            );
            for code in [&native.stdout, &actual] {
                let result = run_with_input(
                    Command::new(ORACLES[index].get().unwrap())
                        .args(["-I", "-S", "-B", "-c", TRACE]),
                    code,
                );
                assert!(
                    result.status.success(),
                    "{python_version}, {optimization_level:?}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
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
        let native = oracle(
            SOURCE.as_bytes(),
            "return_manager_footer.py",
            PythonVersion::Py313,
            optimization_level,
        );
        assert!(
            native.status.success(),
            "{}",
            String::from_utf8_lossy(&native.stderr)
        );
        let actual = compile_with_options(
            SOURCE,
            "return_manager_footer.py",
            &CompileOptions {
                python_version: PythonVersion::Py313,
                optimization_level,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .marshal();
        assert_eq!(actual, native.stdout, "{optimization_level:?}");
        let mut outputs = Vec::new();
        for payload in [&native.stdout, &actual] {
            let result = run_with_input(
                Command::new(ORACLES[1].get().unwrap())
                    .args(["-I", "-S", "-B", "-c", TRACE])
                    .stdout(Stdio::piped()),
                payload,
            );
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            outputs.push(result.stdout);
        }
        assert_eq!(outputs[0], outputs[1], "{optimization_level:?}");
    }
}

#[test]
fn optimized_footer_after_folded_protected_branch_matches_cpython() {
    let source = r"def f(flag, x, y):
    try:
        if True:
            if flag:
                work()
        else:
            other()
        assert flag
    except ValueError:
        recover()
    if x:
        if y:
            work()
        assert y
";
    assert_matches_cpython_versions(
        source,
        "folded_protected_footer.py",
        &LEVELS,
        &[PythonVersion::Py313, PythonVersion::Py314],
    );
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
            let native = oracle(
                SOURCE.as_bytes(),
                "legacy_finally.py",
                python_version,
                optimization_level,
            );
            assert!(
                native.status.success(),
                "{}",
                String::from_utf8_lossy(&native.stderr)
            );
            let actual = compile_with_options(
                SOURCE,
                "legacy_finally.py",
                &CompileOptions {
                    python_version,
                    optimization_level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            assert_eq!(
                actual, native.stdout,
                "{python_version}, {optimization_level:?}"
            );
            for code in [&native.stdout, &actual] {
                let result = run_with_input(
                    Command::new(ORACLES[index].get().unwrap())
                        .args(["-I", "-S", "-B", "-c", TRACE])
                        .stdout(Stdio::piped()),
                    code,
                );
                assert!(
                    result.status.success(),
                    "{python_version}, {optimization_level:?}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
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
                let native = oracle(
                    source.as_bytes(),
                    "legacy_normal_finalizer.py",
                    PythonVersion::Py312,
                    optimization_level,
                );
                assert!(
                    native.status.success(),
                    "{}",
                    String::from_utf8_lossy(&native.stderr)
                );
                let actual = compile_with_options(
                    &source,
                    "legacy_normal_finalizer.py",
                    &CompileOptions {
                        python_version: PythonVersion::Py312,
                        optimization_level,
                        ..CompileOptions::default()
                    },
                )
                .unwrap()
                .marshal();
                assert_eq!(
                    actual, native.stdout,
                    "{optimization_level:?}, {generator}, {has_sibling}"
                );
                for code in [&native.stdout, &actual] {
                    let result = run_with_input(
                        Command::new(ORACLES[0].get().unwrap())
                            .args(["-I", "-S", "-B", "-c", TRACE])
                            .args([
                                if generator { "1" } else { "0" },
                                if has_sibling { "1" } else { "0" },
                            ])
                            .stdout(Stdio::piped()),
                        code,
                    );
                    assert!(
                        result.status.success(),
                        "{optimization_level:?}, {generator}, {has_sibling}: {}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                }
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
                        let native =
                            oracle(source.as_bytes(), "suppressed_assert.py", version, level);
                        assert!(
                            native.status.success(),
                            "{}",
                            String::from_utf8_lossy(&native.stderr)
                        );
                        let actual = compile_with_options(
                            &source,
                            "suppressed_assert.py",
                            &CompileOptions {
                                python_version: version,
                                optimization_level: level,
                                ..CompileOptions::default()
                            },
                        )
                        .unwrap()
                        .marshal();
                        assert_eq!(
                            actual, native.stdout,
                            "{version}, {level:?}, {nested}, {generator}"
                        );
                        if !trace
                            || version != PythonVersion::Py312
                            || level != OptimizationLevel::Zero
                            || !generator
                        {
                            continue;
                        }
                        for code in [&native.stdout, &actual] {
                            let result = run_with_input(
                                Command::new(ORACLES[index].get().unwrap())
                                    .args(["-I", "-S", "-B", "-c", TRACE, &marker.to_string()])
                                    .stdout(Stdio::piped()),
                                code,
                            );
                            assert!(
                                result.status.success(),
                                "{}",
                                String::from_utf8_lossy(&result.stderr)
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn terminal_try_pass_preserves_implicit_return_location() {
    for source in [
        "def f(x):\n    try:\n        x = x + 1\n        pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        del x\n        pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        consume(x)\n        pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        x = x + 1; pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        del x; pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        consume(x); pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        x = (x +\n             1); pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        consume(\n            x,\n        ); pass\n    except ValueError:\n        recover()\n",
        "def f(x, y):\n    try:\n        if y:\n            assert x\n        else:\n            assert y\n        z = x + 1\n        pass\n    except ValueError:\n        recover()\n",
        "def f(x, y):\n    try:\n        if y:\n            assert x\n        else:\n            assert y\n        del x\n        pass\n    except ValueError:\n        recover()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "terminal_try_pass.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn terminal_try_consecutive_passes_preserve_locations() {
    for source in [
        "def f(x):\n    try:\n        x = x + 1\n        pass\n        pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        consume(\n            x,\n        ); pass; pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        x = x + 1\n        pass; pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        x = x + 1\n        pass\n        pass\n        pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try:\n        x = x + 1\n        pass; pass\n        pass\n    except ValueError:\n        recover()\n",
        "def f(x):\n    try: pass; pass\n    except ValueError: recover()\n",
        "def f(x):\n    try:\n        try:\n            consume(x)\n        finally:\n            pass\n        pass\n        pass\n    except ValueError:\n        recover()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "terminal_try_consecutive_passes.py",
            &LEVELS,
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ],
        );
    }

    for source in [
        "def f(x):\n    try:\n        consume(x); pass; pass\n    except ValueError:\n        recover()\n",
        "def f(xs, flag):\n    try:\n        z = [v for v in xs] if flag else []\n        pass\n        pass\n        pass\n    except ValueError:\n        recover()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "terminal_try_consecutive_passes_modern.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn terminal_conditional_comprehension_pass_keeps_return_location() {
    for source in [
        "def f(xs, flag):\n    try:\n        z = [v for v in xs] if flag else []\n        pass\n    except ValueError:\n        recover()\n",
        "def f(xs, flag):\n    try:\n        z = {v for v in xs} if not flag else []; pass\n    except ValueError:\n        recover()\n",
        "def f(xs, flag):\n    try:\n        z = {v: v for v in xs} if check(flag) else []\n        pass\n    except ValueError:\n        recover()\n",
        "def f(xs, flag):\n    try:\n        with manager():\n            pass\n            global other\n        z = [v for v in xs if v] if flag else []\n        pass\n    except ValueError:\n        recover()\n",
        "def f(xs, flag):\n    try:\n        try:\n            pass\n        except TypeError:\n            recover_inner()\n        z = [v for v in xs if v] if flag else []\n        pass\n    except ValueError:\n        recover()\n",
        "def f(xs, flag):\n    try:\n        try:\n            work()\n        finally:\n            pass\n        z = [v for v in xs if v] if flag else []\n        pass\n    except ValueError:\n        recover()\n",
        "def f(xs, flag):\n    global z\n    try:\n        z = [(w := v) for v in xs] if flag else []\n        pass\n    except ValueError:\n        recover()\n",
        "def outer():\n    z = None\n    def f(xs, flag):\n        nonlocal z\n        try:\n            z = [v for v in xs] if flag else []\n            pass\n        except ValueError:\n            recover()\n    return f\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "terminal_conditional_comprehension_pass.py",
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

#[test]
fn terminal_finally_pass_after_nested_function_matches_cpython() {
    for source in [
        "def f(flag):\n    try:\n        def inner():\n            pass\n        result = flag\n        pass\n    finally:\n        cleanup()\n",
        "def f(flag):\n    try:\n        def inner():\n            return flag\n        result = flag\n        pass\n    finally:\n        cleanup(flag)\n",
        "def f(flag):\n    try:\n        def inner():\n            pass\n        result = flag; pass\n    finally:\n        cleanup()\n",
        "def f(flag):\n    try:\n        def inner():\n            'doc'\n            return flag\n        result = (\n            flag); pass\n    finally:\n        cleanup(\n            flag\n        )\n",
        "def f(flag):\n    if False:\n        missing = 1\n    try:\n        def inner():\n            pass\n        result = missing\n        pass\n    finally:\n        cleanup()\n",
        "def f(flag, xs, obj, manager):\n    try:\n        def inner():\n            try:\n                item = [v for v in xs] if flag else []\n                pass\n            except ValueError:\n                work()\n        result = flag\n        pass\n    finally:\n        cleanup()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "terminal_finally_pass.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
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
            let native = oracle(
                SOURCE.as_bytes(),
                "terminal_finally_pass.py",
                version,
                optimization_level,
            );
            assert!(
                native.status.success(),
                "{}",
                String::from_utf8_lossy(&native.stderr)
            );
            let actual = compile_with_options(
                SOURCE,
                "terminal_finally_pass.py",
                &CompileOptions {
                    python_version: version,
                    optimization_level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            for bytes in [&native.stdout[..], &actual] {
                let output = run_with_input(
                    Command::new(ORACLES[index].get().unwrap())
                        .args(["-I", "-S", "-c", TRACE])
                        .stdout(Stdio::piped()),
                    bytes,
                );
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(output.stdout, b"\"ok\"\n");
            }
        }
    }
}

static ORACLES: [OnceLock<OsString>; 4] = [const { OnceLock::new() }; 4];

#[test]
fn optimized_for_loop_backedge_locations_match_cpython_312() {
    for source in [
        "def f(xs):\n for x in xs:\n  if x:\n   assert False\n  if x:\n   assert False\n",
        "def f(xs):\n for x in xs:\n  if x:\n   assert False\n  if x:\n   assert False\n  if x:\n   assert False\n",
        "def f(xs):\n for x in xs:\n  if not x:\n   assert False\n  if not x:\n   assert False\n",
        "def f(xs, pred):\n for x in xs:\n  if pred(x):\n   assert False\n  if pred(x):\n   assert False\n",
        "def f(xs):\n for y in xs:\n  for x in xs:\n   if x:\n    assert False\n   if x:\n    assert False\n",
        "def f(xs):\n for x in xs:\n  if x:\n   assert False\n",
        "def f(xs, y):\n for x in xs:\n  if x and y:\n   assert False\n  if x and y:\n   assert False\n",
        "def f(xs, y):\n for x in xs:\n  if x or y:\n   assert False\n  if x or y:\n   assert False\n",
        "def f(xs):\n for x in xs:\n  if x:\n   assert False\n  if x:\n   assert False\n  pass\n",
        "def f(xs):\n for x in xs:\n  if x:\n   assert False\n  if x:\n   assert False\n  continue\n",
        "def f(x):\n while x:\n  if x:\n   assert False\n  if x:\n   assert False\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "loop_backedge_locations.py",
            &LEVELS,
            &[PythonVersion::Py312],
        );
    }
}

#[test]
fn protected_for_loop_backedge_locations_match_cpython_312() {
    for source in [
        "def f(xs, cleanup):\n try:\n  for x in xs:\n   if x:\n    assert False\n   if x:\n    assert False\n finally:\n  cleanup()\n",
        "def f(xs, manager):\n with manager:\n  for x in xs:\n   if x:\n    assert False\n   if x:\n    assert False\n",
        "def f(xs, x):\n for item in xs:\n  if x:\n   assert False\n  if x:\n   assert False\n try:\n  z = x + 1\n  pass\n except ValueError:\n  recover()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "protected_loop_backedge_locations.py",
            &LEVELS,
            &[PythonVersion::Py312],
        );
    }
}

#[test]
fn preceding_exception_setups_preserve_optimized_backedge_locations() {
    for source in [
        "def f(xs, work, recover):\n try:\n  work()\n except ValueError:\n  recover()\n for x in xs:\n  if x:\n   assert False\n  if x:\n   assert False\n",
        "def f(xs, work, recover):\n try:\n  try:\n   work()\n  except ValueError:\n   recover()\n except Exception:\n  recover()\n for x in xs:\n  if not x:\n   assert False\n  if not x:\n   assert False\n",
        "def f(xs, work, cleanup):\n try:\n  work()\n finally:\n  cleanup()\n for x in xs:\n  if x:\n   assert False\n  if x:\n   assert False\n",
        "def f(xs, work, manager):\n with manager:\n  work()\n for x in xs:\n  if x:\n   assert False\n  if x:\n   assert False\n",
        "def f(xs, work, recover, y):\n try:\n  work()\n except ValueError:\n  recover()\n for x in xs:\n  if x and y:\n   assert False\n  if x and y:\n   assert False\n  work()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "preceding_exception_backedge.py",
            &LEVELS,
            &[PythonVersion::Py312],
        );
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
        let native = oracle(
            SOURCE.as_bytes(),
            "preceding_exception_trace.py",
            PythonVersion::Py312,
            level,
        );
        assert!(native.status.success());
        let actual = compile_with_options(
            SOURCE,
            "preceding_exception_trace.py",
            &CompileOptions {
                python_version: PythonVersion::Py312,
                optimization_level: level,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .marshal();
        assert_eq!(actual, native.stdout);
        for payload in [&native.stdout, &actual] {
            let result = run_with_input(
                Command::new(ORACLES[0].get().unwrap()).args(["-I", "-S", "-B", "-c", TRACE]),
                payload,
            );
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
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
        let native = oracle(
            SOURCE.as_bytes(),
            "generator_backedge.py",
            PythonVersion::Py312,
            level,
        );
        assert!(native.status.success());
        let actual = compile_with_options(
            SOURCE,
            "generator_backedge.py",
            &CompileOptions {
                python_version: PythonVersion::Py312,
                optimization_level: level,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .marshal();
        assert_eq!(actual, native.stdout);
        for payload in [&native.stdout, &actual] {
            let result = run_with_input(
                Command::new(ORACLES[0].get().unwrap()).args(["-I", "-S", "-B", "-c", TRACE]),
                payload,
            );
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}

#[test]
fn generator_backedge_prologues_match_cpython_312() {
    for source in [
        "def enclosing():\n value = 1\n def f(xs, work, recover):\n  try:\n   work()\n  except ValueError:\n   recover()\n  for x in xs:\n   if x:\n    assert False\n   if x:\n    assert False\n  yield value\n return f\n",
        "def f(xs, work, recover):\n value = 1\n def g():\n  return value\n try:\n  work()\n except ValueError:\n  recover()\n for x in xs:\n  if x:\n   assert False\n  if x:\n   assert False\n yield g\n",
        "def f(xs):\n return (x for x in xs if x)\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "generator_prologue.py",
            &LEVELS,
            &[PythonVersion::Py312],
        );
    }
}

#[test]
fn terminating_if_before_elided_assert_preserves_branch_location() {
    for source in [
        "def f(a,b,c):\n if a:\n  if b:\n   raise ValueError()\n  assert a\n else:\n  other()\n after()\n",
        "def f(a,b,c):\n if a:\n  if not b:\n   raise ValueError()\n  assert a\n else:\n  other()\n after()\n",
        "def f(a,b,c):\n if a:\n  if b or c:\n   raise ValueError()\n  assert a\n else:\n  other()\n after()\n",
        "def f(a,b,c):\n if a:\n  if b < c:\n   return 3\n  assert a\n else:\n  other()\n after()\n",
        "def f(a,b,c):\n if a:\n  other()\n elif c:\n  if check(b):\n   return work(b)\n  assert a\n  assert (z := b)\n else:\n  other()\n after()\n",
        "def f(a,b,c):\n if a:\n  if b:\n   raise ValueError()\n  assert a\n  pass\n else:\n  other()\n after()\n",
        "def f(a,b,c):\n if a:\n  if True:\n   raise ValueError()\n  assert a\n else:\n  other()\n after()\n",
        "def f(a,b,c,cm,xs):\n if a:\n  try:\n   work()\n  finally:\n   cleanup()\n  if b:\n   raise ValueError()\n  assert a\n else:\n  other()\n after()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/elided_terminating_if.py",
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

#[test]
fn nonterminal_elided_match_case_preserves_unlocated_exit() {
    for source in [
        "def f(x):\n match x:\n  case str():\n   assert x\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
        "def f(x):\n match x:\n  case None:\n   pass\n  case str() | bytes():\n   assert x\n   assert (w := x)\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
        "def f(x):\n match x:\n  case str(v) | bytes(v):\n   assert (z := x)\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
        "def f(x):\n match x:\n  case A(foo=y) | B(foo=y):\n   assert x\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
        "def f(x):\n match x:\n  case (1, 2) | (3, 4):\n   assert x\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
        "def f(x):\n global z\n match x:\n  case str() | bytes():\n   assert (z := x)\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
        "def f(x):\n match x:\n  case str():\n   assert x\n   pass\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
        "def f(x):\n match x:\n  case str():\n   assert x\n   work()\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/nonterminal_elided_match.py",
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

#[test]
fn nonterminal_elided_match_in_suspended_function_matches_cpython() {
    for source in [
        "async def f(x):\n await work()\n match x:\n  case str():\n   assert x\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
        "async def f(x):\n match x:\n  case str() | bytes():\n   assert (z := x)\n  case int():\n   work()\n  case _:\n   other()\n await work()\n",
        "def f(x):\n yield x\n match x:\n  case str() | bytes():\n   assert x\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
        "def f(x):\n match x:\n  case str(v) | bytes(v):\n   assert (z := x)\n  case int():\n   work()\n  case _:\n   other()\n yield x\n",
        "async def f(x):\n yield x\n match x:\n  case str() | bytes():\n   assert x\n  case int():\n   work()\n  case _:\n   other()\n after()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/nonterminal_elided_match_suspended.py",
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
fn elided_inner_else_keeps_for_loop_backedge_location() {
    for source in [
        "def f(xs,a,b):\n for x in xs:\n  if a:\n   if b:\n    z = x\n   else:\n    assert b\n  else:\n   other(x)\n",
        "def f(xs,a,b,c):\n for x in xs:\n  if a:\n   other(x)\n  elif c:\n   if not b:\n    work(x)\n   else:\n    assert b\n    assert (w := c)\n  else:\n   other(x)\n",
        "def f(xs,a,b,c):\n for x in xs:\n  if a:\n   if b < c:\n    z += 1\n   else:\n    assert b\n  else:\n   other(x)\n",
        "def f(xs,a,b,c):\n for x in xs:\n  if a:\n   if b or c:\n    z = x\n   else:\n    assert b\n  else:\n   other(x)\n",
        "def f(xs,a,b,c):\n for x in xs:\n  if c:\n   if a:\n    work(x)\n   elif b:\n    z = x\n   else:\n    assert b\n  else:\n   other(x)\n",
        "def f(xs,a,b):\n global z\n for x in xs:\n  if a:\n   if b:\n    z = x\n   else:\n    assert b\n  else:\n   other(x)\n",
        "def f(xs,a,b):\n def read():\n  return z\n for x in xs:\n  if a:\n   if b:\n    z = x\n   else:\n    assert b\n  else:\n   other(x)\n",
        "def f(xs,a,b):\n for x in xs:\n  if a:\n   if b:\n    z = x\n   else:\n    assert b\n    pass\n  else:\n   other(x)\n",
        "def f(xs,a,b):\n for x in xs:\n  if a:\n   if True:\n    z = x\n   else:\n    assert b\n  else:\n   other(x)\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/elided_inner_else_loop.py",
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
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ],
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
            let native = oracle(
                SOURCE.as_bytes(),
                "elided_inner_else_for_else.py",
                python_version,
                optimization_level,
            );
            assert!(native.status.success());
            let actual = compile_with_options(
                SOURCE,
                "elided_inner_else_for_else.py",
                &CompileOptions {
                    python_version,
                    optimization_level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            assert_eq!(actual, native.stdout);
            for payload in [&native.stdout, &actual] {
                let result = run_with_input(
                    Command::new(ORACLES[index].get().unwrap()).args(["-I", "-S", "-c", TRACE]),
                    payload,
                );
                assert!(
                    result.status.success(),
                    "{python_version:?} {optimization_level:?}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
        }
    }
}

#[test]
fn elided_inner_else_keeps_protected_loop_backedge_location() {
    for source in [
        "def f(a,b,c,ys):\n try:\n  for v in ys:\n   if a:\n    if b:\n     z = v\n    else:\n     assert b\n   else:\n    consume(v)\n except ValueError:\n  recover()\n",
        "def f(a,b,c,ys):\n try:\n  for v in ys:\n   if a:\n    if b or c:\n     consume(v)\n    else:\n     assert b\n   else:\n    consume(v)\n  after = a\n except ValueError:\n  recover()\n",
        "def f(a,b,c,ys):\n try:\n  for v in ys:\n   if a:\n    if not b:\n     consume(v)\n    else:\n     assert b\n   else:\n    consume(v)\n  z = a + 1\n  pass\n  pass\n except ValueError:\n  recover()\n",
        "def f(a,b,c,ys):\n try:\n  for v in ys:\n   if a:\n    if b < c:\n     z = v\n    else:\n     assert b\n   else:\n    consume(v)\n  after = a\n except ValueError:\n  recover()\n else:\n  consume(0)\n",
        "def f(a,b,c,items):\n    try:\n        for item in items:\n            if a:\n                if b:\n                    consume(item)\n                elif c:\n                    consume(item)\n                else:\n                    assert b\n            else:\n                consume(item)\n        z = 1\n    except ValueError:\n        recover()\n",
        "def f(a,b,c,items):\n    try:\n        for item in items:\n            if a:\n                if b:\n                    consume(item)\n                elif c:\n                    assert c\n                else:\n                    assert b\n            else:\n                consume(item)\n        else:\n            finish()\n        z = 1\n    except ValueError:\n        recover()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/protected_elided_inner_else_loop.py",
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

#[test]
fn shared_match_exit_keeps_each_return_location() {
    for source in [
        r"def enclosing():
    cell = 1
    async def f(value, flag, work):
        if flag:
            match value:
                case int():
                    result = 1
                case str():
                    result = 2
                case None:
                    result = 3
            assert flag
    return f
f = enclosing()
",
        r"def enclosing():
    cell = 1
    def f(value, flag, work):
        if flag:
            match value:
                case int() if flag:
                    result = 1
                case str() | bytes():
                    result = 2
                case None:
                    result = 3
            global marker
            assert flag
        else:
            work()
    return f
f = enclosing()
",
        r"def enclosing():
    cell = 1
    def f(value, flag, work):
        yield 1
        if flag:
            match value:
                case 1:
                    result = 1
                case 2:
                    result = 2
                case _:
                    result = 3
            nonlocal cell
            assert flag
    return f
f = enclosing()
",
        r"def enclosing():
    cell = 1
    async def f(value, flag, work):
        yield 1
        if flag:
            match value:
                case int():
                    result = 1
                case str():
                    result = 2
                case None:
                    result = 3
            assert flag
        else:
            work()
    return f
f = enclosing()
",
        r"def enclosing():
    cell = 1
    def f(value, flag, work):
        if flag:
            match value:
                case int():
                    result = 1
                case str():
                    result = 2
                case None:
                    result = 3
            work()
    return f
f = enclosing()
",
        r#"def enclosing():
    def f(value, flag, work, manager):
        try:
            if flag:
                match value:
                    case int():
                        return
                    case str():
                        result = 2
                global marker
                assert flag
        except Exception:
            work("caught")
    return f
f = enclosing()
"#,
        r"def enclosing():
    def f(value, flag, work, manager):
        with manager:
            if flag:
                match value:
                    case int():
                        result = 1
                global marker
                assert flag
    return f
f = enclosing()
",
        r"def enclosing():
    def f(value, flag, work, manager):
        if flag:
            match value:
                case _:
                    result = 1
            global marker
            assert flag
    return f
f = enclosing()
",
    ] {
        assert_matches_cpython(source, "sub/shared_match_return.py", &LEVELS);
    }
}

#[test]
fn nested_pass_before_elided_assert_preserves_return_locations() {
    for source in [
        "def f(a,b):\n if a:\n  if b:\n   pass\n  assert a\n",
        "def f(a,b,c):\n if a:\n  if b and c:\n   pass\n  assert a\n",
        "def f(a,b):\n if a:\n  if b:\n   pass\n  else:\n   assert b\n  assert a\n",
        "def f(a,b):\n if a:\n  if b:\n   pass\n  assert a\n else:\n  work()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/nested_pass_elided.py",
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
fn elided_assertions_preserve_nested_assignment_join_nop() {
    assert_matches_cpython_versions(
        "def f(flag, item):\n    if flag:\n        if item is None:\n            value = 1.0\n        else:\n            value = convert(item)\n        assert value\n    else:\n        value = 2.0\n    return value\n",
        "nested_assignment_join.py",
        &LEVELS,
        &[PythonVersion::Py314, PythonVersion::Py315],
    );
}

#[test]
fn nested_terminal_if_before_elided_assert_matches_cpython() {
    for source in [
        "def f(a,b):\n if a:\n  if b:\n   x = 1\n  assert a\n",
        "def f(a,b):\n if a:\n  if b:\n   assert a\n  assert b\n",
        "def f(a,b,c):\n if a:\n  if b and c:\n   assert b\n  assert a\n",
        "def f(a,b):\n if a:\n  if b:\n   assert (x := b)\n   assert (y := a)\n  assert a\n",
        "def f(a,b):\n def read():\n  return x\n if a:\n  if b:\n   assert (x := b)\n  assert a\n",
        "def f(a,b):\n if a:\n  if b:\n   work()\n  assert a\n  assert b\n",
        "def f(a,b):\n if a:\n  if b:\n   work()\n  else:\n   assert b\n  assert a\n",
        "def f(a,b):\n if a:\n  if b:\n   work()\n  else:\n   assert (x := a)\n  assert b\n",
        "def f(a,b):\n def read():\n  return x\n if a:\n  if b:\n   work()\n  else:\n   assert (x := a)\n  assert b\n",
        "def f(a,b):\n global x\n if a:\n  if b:\n   work()\n  else:\n   assert (x := a)\n  assert b\n",
        "def outer():\n x=0\n def f(a,b):\n  nonlocal x\n  if a:\n   if b:\n    work()\n   else:\n    assert (x := a)\n   assert b\n return f\n",
        "def f(a,b,c):\n if a:\n  if b and c:\n   assert a\n  else:\n   assert b\n  assert c\n",
        "def f(a,b):\n global x\n if a:\n  if b:\n   x = 1\n  else:\n   assert a\n  assert b\n",
        "def f(a,b,x):\n if a:\n  if not b:\n   x += 1\n  assert a\n",
        "def f(a,b):\n global x\n if a:\n  if b:\n   x = 1\n  assert a\n",
        "def outer():\n x=0\n def f(a,b):\n  nonlocal x\n  if a:\n   if b:\n    x = 1\n   assert a\n return f\n",
        "def f(a,b,obj):\n if a:\n  if b:\n   left, *rest = obj\n  assert a\n else:\n  work()\n",
        "def f(a,b):\n if a:\n  if b:\n   x = 1\n  assert a\n  pass\n",
        "def f(a,b,cm):\n with cm:\n  if a:\n   if b:\n    x = 1\n   assert a\n",
        "def f(a,b):\n if a:\n  if b:\n   x = 1\n  assert a\n yield b\n",
        "async def f(a,b):\n if a:\n  if b:\n   x = 1\n  assert a\n await work()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/nested_terminal_elided.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn nested_loop_if_before_elided_assert_matches_cpython() {
    for source in [
        "def f(xs,a,b):\n for x in xs:\n  if a:\n   if b:\n    work(x)\n   assert x\n",
        "def f(xs,a,b):\n for x in xs:\n  if a:\n   if b:\n    assert b\n   assert x\n",
        "def f(xs,a,b):\n for x in xs:\n  if a:\n   if b:\n    a += 1\n   assert x\n",
        "def f(xs,a,b):\n while xs:\n  x=xs.pop()\n  if a:\n   if b:\n    work(x)\n   assert x\n",
        "def f(xs,a,b):\n for x in xs:\n  if a:\n   if b:\n    work(x)\n   assert x\n  else:\n   other(x)\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/nested_loop_elided.py",
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
fn terminal_if_before_elided_assert_matches_cpython_312() {
    for source in [
        "def f(x):\n if x:\n  with context():\n   call()\n else:\n  if x:\n   assert x\n  assert x\n",
        "def f(x, y):\n    if x:\n        with context():\n            call()\n    else:\n        if y:\n            pass\n        assert x\n        call()\n",
        "def f(x, y):\n    if x:\n        with context():\n            call()\n    else:\n        if x:\n            assert x\n        assert x\n        return 1\n",
        "def f(x, y):\n    if x:\n        with context():\n            call()\n    else:\n        if x:\n            assert x\n        assert x\n        raise ValueError\n",
        "def f(x, y):\n    if x:\n        with context():\n            call()\n    else:\n        if x:\n            assert x\n        assert x\n        try:\n            call()\n        finally:\n            clean()\n",
        "def f(x):\n if x:\n  call()\n else:\n  with context():\n   call()\n  if x:\n   assert x\n  assert x\n",
        "def f(x):\n if x:\n  with context():\n   call()\n else:\n  if x:\n   assert x\n",
        "def f(x):\n with context():\n  call()\n if x:\n  assert x\n assert x\n",
        "def f(x):\n if x:\n  call()\n else:\n  if x:\n   assert x\n  assert x\n",
        "def f(x):\n if x:\n  assert x\n assert x\n",
        "def f(x):\n if x:\n  try:\n   call()\n  except Exception:\n   pass\n else:\n  if x:\n   assert x\n  assert x\n",
        "def f(x):\n if x:\n  while x:\n   call()\n else:\n  if x:\n   assert x\n  assert x\n",
        "def f(x):\n if x:\n  for a in x:\n   call()\n else:\n  if x:\n   assert x\n  assert x\n",
        "def f(x):\n if x:\n  return 3\n else:\n  if x:\n   assert x\n  assert x\n",
        "def f(x):\n try:\n  if x:\n   with context():\n    call()\n  else:\n   if x:\n    assert x\n   assert x\n except Exception:\n  recover()\n",
        "def f(x):\n try:\n  if x:\n   with context():\n    call()\n  else:\n   if x:\n    assert x\n   assert x\n finally:\n  clean()\n",
        "def f(x):\n with context():\n  if x:\n   with context():\n    call()\n  else:\n   if x:\n    assert x\n   assert x\n",
        "def f(x):\n if x:\n  if x:\n   assert x\n  assert x\n else:\n  with context():\n   call()\n",
        "def f(x):\n if x:\n  with context():\n   call()\n else:\n  if x:\n   assert x\n  assert x\n  assert x\n",
        "async def f(x):\n if x:\n  with context():\n   call()\n else:\n  if x:\n   assert x\n  assert x\n",
        "def f(x):\n if x:\n  with context():\n   yield x\n else:\n  if x:\n   assert x\n  assert x\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "terminal_elided_assert.py",
            &LEVELS,
            &[PythonVersion::Py312],
        );
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
        let native = oracle(
            SOURCE.as_bytes(),
            "terminal_elided_assert.py",
            PythonVersion::Py312,
            optimization_level,
        );
        assert!(native.status.success());
        let actual = compile_with_options(
            SOURCE,
            "terminal_elided_assert.py",
            &CompileOptions {
                python_version: PythonVersion::Py312,
                optimization_level,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .marshal();
        let execute = |bytes: &[u8]| {
            run_with_input(
                Command::new(ORACLES[0].get().unwrap())
                    .args(["-I", "-S", "-c", TRACE])
                    .stdout(Stdio::piped()),
                bytes,
            )
        };
        for result in [execute(&native.stdout), execute(&actual)] {
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(
                result.stdout,
                b"[\"enter\", \"exit:ValueError\", \"escaped\"]\n"
            );
        }
    }
}

#[test]
fn elided_boolean_loop_tail_after_prefix_or_nested_condition() {
    for source in [
        r"
def f(items):
    for item in items:
        first, second = item
        if first == 1 or second == 2:
            assert first
",
        r"
def f(items):
    assert items
    values = prepare(items)
    for item in values:
        first, second = item, item
        assert first
        if first == 1 or second == 2:
            assert second
",
        r"
def f(items):
    for item in items:
        if condition(item):
            if item is not None:
                if item in (1, 2) or item in (3, 4):
                    assert item
",
    ] {
        assert_matches_cpython(source, "elided_boolean_loop_tail.py", &LEVELS);
    }
}

#[test]
fn elided_boolean_loop_tail_after_continue_guard() {
    for source in [
        r"
def f(items, a, b, c, d):
    for item in items:
        if item <= 0:
            continue
        if a != b or c != d:
            assert item
",
        r"
def f(groups, a, b, c, d):
    for items in groups:
        for item in items:
            if item is None:
                continue
            value = transform(item)
            if a != b or c != d:
                assert value
        action()
",
    ] {
        assert_matches_cpython(source, "elided_loop_tail_continue.py", &LEVELS);
    }
}

#[test]
fn non_final_loop_branches_with_elided_else_keep_copied_exits() {
    for source in [
        r"
def f(a,b,c):
    for x in a:
        if c:
            if a and b:
                assert a
            else:
                assert b
        else:
            a=b
",
        r"
def f(items,a,b,c,d,obj):
    for x in items:
        if c:
            if a and b:
                obj.action()
            else:
                assert b
        else:
            a=d
",
        r"
def f(items,a,b,c,d,obj):
    for x in items:
        if c:
            if a:
                obj.action()
            elif a and b:
                obj.action()
            else:
                assert b
        else:
            a=d
",
        r"
def f(items,a,b,c,d,obj):
    while items:
        if c:
            if a and b:
                obj.action()
            else:
                assert b
        else:
            a=d
",
        r"
def f(a,b,c):
    for x in a:
        if c:
            if a and b:
                pass
            else:
                assert b
        else:
            a=b
",
        r"
def f(items,a,b,c,d,obj,cm):
    for x in items:
        if c:
            if a or b:
                obj.action()
            else:
                assert a
        else:
            obj.action()
    else:
        obj.action()
",
    ] {
        assert_matches_cpython(source, "non_final_elided_loop_else.py", &LEVELS);
    }
}

#[test]
fn loop_tail_elided_else_matches_cpython() {
    for source in [
        r"
def f(xs):
    n = None
    for count in xs:
        if n is None:
            n = count
        else:
            assert n == count
    return n
",
        r"
def f(xs, a, b):
    for x in xs:
        if a and b:
            work(x)
        else:
            assert x
",
        r"
def f(xs):
    for x in xs:
        if False:
            work(x)
        else:
            assert x
",
        r"
def f(xs):
    for x in xs:
        if x:
            work(x)
        else:
            assert x
            pass
",
        r"
def f(xs):
    for x in xs:
        if x:
            work(x)
        else:
            assert x
            work_else(x)
",
    ] {
        assert_matches_cpython(source, "elided_loop_else.py", &LEVELS);
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
fn elided_loop_arms_preserve_forward_continuation_nops() {
    for source in [
        "def f(cm, items):\n    for item in items:\n        with cm:\n            if item:\n                assert item\n            else:\n                assert items\n",
        "def f(cm, items):\n    with cm:\n        for item in items:\n            if item:\n                assert item\n            else:\n                assert items\n            work(item)\n",
        "async def f(items):\n    for item in items:\n        if item:\n            assert item\n        else:\n            assert items\n        work(item)\n",
        "def f(items):\n    for item in items:\n        if item:\n            assert item\n        else:\n            assert items\n        value = item\n    yield value\n",
    ] {
        assert_matches_cpython(source, "elided_loop_forward_join.py", &LEVELS);
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
fn elided_loop_tail_else_preserves_nested_and_suspended_branches() {
    for source in [
        r"
def f(xs, flag):
    for x in xs:
        if x:
            if flag:
                work(x)
        else:
            assert x
",
        r"
def f(xs, flag):
    for x in xs:
        if x:
            work(x)
        elif flag:
            if x > 1:
                work_else(x)
        else:
            assert x
",
        r"
def f(xs):
    for x in xs:
        if x:
            yield x
        else:
            assert x
",
    ] {
        assert_matches_cpython(source, "nested_elided_loop_else.py", &LEVELS);
    }
}

#[test]
fn protected_loop_tail_controls_match_cpython() {
    for source in [
        r"
def f(xs):
    try:
        for x in xs:
            if x:
                work(x)
            else:
                assert x
    except ValueError:
        cleanup()
",
        r"
def f(xs):
    for x in xs:
        try:
            if x:
                work(x)
            else:
                assert x
        except ValueError:
            cleanup()
",
        r"
def f(xs):
    try:
        for x in xs:
            if x:
                work(x)
            else:
                assert x
    finally:
        cleanup()
",
        r"
def f(xs, cm):
    with cm:
        for x in xs:
            if x:
                work(x)
            else:
                assert x
",
    ] {
        assert_matches_cpython(source, "protected_loop_tail.py", &LEVELS);
    }
}

#[test]
fn elided_if_else_preserves_conditional_join() {
    for source in [
        "def f(a):\n if a:\n  assert x\n else:\n  assert y\n work(a)\n return 42\n",
        "def f(a,b):\n if a and b:\n  assert x\n else:\n  assert y\n work(a)\n",
        "def f(a,b):\n if a or b:\n  assert x\n else:\n  assert y\n work(a)\n",
        "def f(a):\n if not __debug__:\n  assert x\n else:\n  assert y\n work(a)\n",
    ] {
        assert_matches_cpython(source, "sub/elided_if_else.py", &LEVELS);
    }
}

#[test]
fn elided_elif_else_preserves_conditional_join() {
    for source in [
        "def f(first, second):\n if first:\n  assert first\n elif second:\n  assert second\n else:\n  assert first\n action()\n",
        "def f(first, second):\n if first:\n  assert first\n elif first or second:\n  assert second\n else:\n  assert first\n action()\n",
        "def f(first, second):\n if first:\n  assert first\n elif second:\n  assert second\n else:\n  assert first\n return 1\n",
        "def f(first, second):\n if first:\n  assert first\n elif second:\n  assert second\n action()\n",
    ] {
        assert_matches_cpython(source, "sub/elided_elif_else.py", &LEVELS);
    }
}

#[test]
fn protected_elided_if_else_preserves_conditional_join() {
    for source in [
        "def f(a):\n try:\n  if a:\n   assert a\n  else:\n   assert not a\n  work(a)\n except ValueError:\n  recover()\n",
        "def f(a):\n try:\n  if a is None:\n   assert a\n  else:\n   assert not a\n  work(a)\n except ValueError:\n  recover()\n else:\n  finish()\n",
        "def f(a, b):\n try:\n  if a:\n   assert a\n  else:\n   assert b\n  assert b\n except ValueError:\n  recover()\n",
        "def f(a, b):\n try:\n  if a or b:\n   assert a\n  else:\n   assert b\n  work(a)\n except ValueError:\n  recover()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/protected_elided_if_else.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn elided_outer_else_preserves_nested_conditional_join() {
    for source in [
        "def f(a,b):\n if a:\n  if b:\n   assert a\n  else:\n   assert b\n else:\n  assert a\n work()\n",
        "def f(a,b):\n if a:\n  if b:\n   assert a\n  else:\n   assert b\n else:\n  assert a\n return 1\n",
        "def f(a,b):\n if a:\n  if b:\n   assert a\n  else:\n   assert b\n work()\n",
        "def f(a,b):\n if a:\n  if b:\n   x = 1\n  else:\n   x = 2\n else:\n  assert a\n work(x)\n",
        "def f(a,b):\n if a and b:\n  if not b:\n   x = 1\n  else:\n   x = 2\n else:\n  assert a\n work(x)\n",
        "def f(a,b):\n if a:\n  if b:\n   work(1)\n  else:\n   work(2)\n else:\n  assert a\n finish()\n",
        "def f(a,b):\n if a:\n  if b:\n   x = 1\n  else:\n   x = 2\n else:\n  assert a\n  assert b\n work(x)\n",
        "def f(a,b):\n if a:\n  if b:\n   x = 1\n  else:\n   x = 2\n else:\n  assert a\n  work(a)\n finish()\n",
    ] {
        assert_matches_cpython(source, "sub/elided_nested_if.py", &LEVELS);
    }
}

#[test]
fn protected_elided_if_else_preserves_assignment_and_delete_join() {
    for source in [
        "def f(a):\n try:\n  if a:\n   assert a\n  else:\n   assert not a\n  a = a + 1\n except ValueError:\n  recover()\n",
        "def f(a):\n try:\n  if a:\n   assert a\n  else:\n   assert not a\n  del a\n except ValueError:\n  recover()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/protected_elided_assignment_delete.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn elided_if_else_in_coroutines_and_with_matches_cpython_315() {
    for source in [
        "async def f(flag):\n if flag is not False:\n  assert flag\n else:\n  assert not flag\n await work()\n",
        "def f(flag):\n with manager():\n  if flag == 'v1':\n   assert flag\n  else:\n   assert not flag\n",
        "def f(flag):\n with manager():\n  if flag:\n   assert flag\n  else:\n   assert not flag\n  work()\n",
        "async def f(flag):\n async with manager():\n  if flag:\n   assert flag\n  else:\n   assert not flag\n  await work()\n",
        "def f(flag):\n if flag:\n  assert flag\n else:\n  assert not flag\n yield flag\n",
        "def f(a,b):\n with manager():\n  if a and b:\n   assert a\n  else:\n   assert b\n  work()\n",
        "with manager():\n if flag:\n  assert flag\n else:\n  assert not flag\n work()\n",
        "class C:\n with manager():\n  if flag:\n   assert flag\n  else:\n   assert not flag\n  work()\n",
        "def f(flag):\n with manager():\n  if flag:\n   assert flag\n  else:\n   assert not flag\n  yield 1\n  raise StopIteration()\n",
        "async def f(flag):\n async with manager():\n  if flag:\n   assert flag\n  else:\n   assert not flag\n  yield 1\n  raise StopAsyncIteration()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "protected_elided_if_else.py",
            &LEVELS,
            &[PythonVersion::Py315],
        );
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
        let native = oracle(SOURCE.as_bytes(), "optimized_join.py", version, level);
        assert!(native.status.success());
        let actual = compile_with_options(
            SOURCE,
            "optimized_join.py",
            &CompileOptions {
                python_version: version,
                optimization_level: level,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .marshal();
        assert_eq!(actual, native.stdout);
        let index = VERSIONS
            .iter()
            .position(|(target, _, _)| *target == version)
            .unwrap();
        for (label, payload) in [
            ("native", native.stdout.as_slice()),
            ("serc", actual.as_slice()),
        ] {
            let result = run_with_input(
                Command::new(ORACLES[index].get().unwrap())
                    .args(["-I", "-S", "-X", "utf8", "-c", TRACE_JOIN])
                    .stdout(Stdio::piped()),
                payload,
            );
            assert!(
                result.status.success(),
                "{level:?}, {label}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}

#[test]
fn protected_comprehension_before_elided_assertion_keeps_return_locations() {
    for source in [
        "def f(a, ys):\n try:\n  if a:\n   x = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, ys):\n try:\n  x = 3\n  if a:\n   x = {i for i in ys}\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, ys):\n try:\n  if a:\n   x = {i: i for i in ys}\n  assert a\n except:\n  work()\n",
        "def f(a, b, x, ys):\n try:\n  if a:\n   assert a\n  else:\n   assert b\n  x = 3\n  if b:\n   x = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/protected_comprehension_assert.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
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
                let native = oracle(source.as_bytes(), "protected_owner.py", version, level);
                assert!(native.status.success());
                let actual = compile_with_options(
                    source,
                    "protected_owner.py",
                    &CompileOptions {
                        python_version: version,
                        optimization_level: level,
                        ..CompileOptions::default()
                    },
                )
                .unwrap()
                .marshal();
                assert_eq!(actual, native.stdout, "{version}, {level:?}");
                let index = VERSIONS
                    .iter()
                    .position(|(candidate, _, _)| *candidate == version)
                    .unwrap();
                for (label, payload) in [("native", &native.stdout), ("serc", &actual)] {
                    let result = run_with_input(
                        Command::new(ORACLES[index].get().unwrap())
                            .args(["-I", "-S", "-B", "-c", TRACE])
                            .args(&lines)
                            .stdout(Stdio::piped()),
                        payload,
                    );
                    assert!(
                        result.status.success(),
                        "{version}, {level:?}, {label}: {}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                }
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
    let versions = [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ];
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
                let native = oracle(source.as_bytes(), "folded_pass_owner.py", version, level);
                assert!(native.status.success());
                let actual = compile_with_options(
                    &source,
                    "folded_pass_owner.py",
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
                for (label, bytes) in [("native", &native.stdout), ("serc", &actual)] {
                    let result = run_with_input(
                        Command::new(ORACLES[index].get().unwrap())
                            .args(["-I", "-S", "-B", "-c", TRACE, target_line, argument])
                            .stdout(Stdio::piped()),
                        bytes,
                    );
                    assert!(
                        result.status.success(),
                        "{version}, {level:?}, {label}: {}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                }
            }
        }
    }
}

const ORACLE_IDENTITY: &str = r"
import importlib.util, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
";
const MARSHAL_ORACLE: &str = r"
import marshal, sys
code = compile(sys.stdin.buffer.read(), sys.argv[2], 'exec', dont_inherit=True, optimize=int(sys.argv[1]))
sys.stdout.buffer.write(marshal.dumps(code))
";

fn run_with_input(command: &mut Command, input: &[u8]) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

fn oracle(
    source: &[u8],
    filename: &str,
    version: PythonVersion,
    level: OptimizationLevel,
) -> Output {
    let index = VERSIONS
        .iter()
        .position(|(candidate, _, _)| *candidate == version)
        .unwrap();
    let executable = ORACLES[index].get_or_init(|| {
        let (_, variable, fallback) = VERSIONS[index];
        let executable = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let target = version.target();
        let identity = Command::new(&executable)
            .args(["-I", "-S", "-c", ORACLE_IDENTITY])
            .args([target.tag.trim_start_matches('v'), &target.magic_hex()])
            .output()
            .expect("check the pinned CPython oracle");
        assert!(
            identity.status.success(),
            "{variable} must name {} with magic {}: {}",
            target.tag,
            target.magic_hex(),
            String::from_utf8_lossy(&identity.stderr)
        );
        executable
    });
    let level = match level {
        OptimizationLevel::Zero => "0",
        OptimizationLevel::One => "1",
        OptimizationLevel::Two => "2",
    };
    let mut child = Command::new(executable)
        .args(["-I", "-S", "-X", "utf8", "-c", MARSHAL_ORACLE])
        .args([level, filename])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the pinned CPython oracle");
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}

fn assert_matches_cpython(source: &str, filename: &str, levels: &[OptimizationLevel]) {
    let versions = VERSIONS.map(|(version, _, _)| version);
    assert_matches_cpython_versions(source, filename, levels, &versions);
}

fn assert_matches_cpython_versions(
    source: &str,
    filename: &str,
    levels: &[OptimizationLevel],
    versions: &[PythonVersion],
) {
    for &python_version in versions {
        for &optimization_level in levels {
            let options = CompileOptions {
                python_version,
                optimization_level,
                ..CompileOptions::default()
            };
            let expected = oracle(
                source.as_bytes(),
                filename,
                python_version,
                optimization_level,
            );
            assert!(
                expected.status.success(),
                "{python_version}, {optimization_level:?}, {filename}: {}",
                String::from_utf8_lossy(&expected.stderr)
            );
            let actual = compile_with_options(source, filename, &options)
                .unwrap_or_else(|error| {
                    panic!("{python_version}, {optimization_level:?}, {filename}: {error}")
                })
                .marshal();
            let first_difference = actual
                .iter()
                .zip(&expected.stdout)
                .position(|(actual, expected)| actual != expected);
            assert!(
                actual == expected.stdout,
                "{python_version}, {optimization_level:?}, {filename}: first difference \
                 {first_difference:?}, lengths {} and {}",
                actual.len(),
                expected.stdout.len()
            );
        }
    }
}

#[test]
fn assertions_match_cpython_at_every_optimization_level() {
    assert_matches_cpython(
        r"
assert ready, describe()
def check(value):
    assert value
    assert value > 0, f'bad value: {value}'
    return value
def only_assert():
    assert side_effect(), message()
class C:
    assert ready
    def method(self):
        assert self.ready
try:
    effect()
finally:
    assert ready
try:
    assert ready
except ValueError:
    assert recovery
try:
    assert ready
except:
    assert recovery
after = 1
try:
    assert ready
except:
    assert recovery
finally:
    after = 2
",
        "optimized_assertions.py",
        &LEVELS,
    );
}

#[test]
fn removed_assertions_preserve_scope_and_generator_metadata() {
    assert_matches_cpython(
        r"
def assigned():
    assert (local := value)
    return local
def outer(captured):
    def inner():
        assert captured
    return inner
def captured_by_assert(local):
    assert lambda: local
def generator():
    assert (yield value)
def delegated():
    assert (yield from values)
async def async_generator():
    assert (yield value)
async def coroutine():
    assert await value
",
        "optimized_assertion_scopes.py",
        &LEVELS,
    );
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
fn debug_constants_match_cpython_through_nested_expressions_and_scopes() {
    assert_matches_cpython(
        r"
debug = __debug__
constants = (__debug__, not __debug__, (__debug__,))
choice = 'debug' if __debug__ else 'optimized'
if __debug__:
    branch = 1
else:
    branch = 2
def function(first):
    while __debug__:
        break
    if not __debug__:
        first += 1
    return __debug__
def conjunction(value):
    return __debug__ and value
def disjunction(value):
    return __debug__ or value
class C:
    debug = __debug__
    def method(self, default=__debug__):
        return lambda: (__debug__, default)
values = [__debug__ for item in items if __debug__ or item]
generator = (__debug__ for item in items if not __debug__)
",
        "optimized_debug.py",
        &LEVELS,
    );
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
fn removed_assertion_comprehensions_do_not_allocate_temporary_locals() {
    assert_matches_cpython(
        r"
assert [item for item in values]
class C:
    assert [item for item in values]
def check(values):
    assert [item for item in values]
    after = 1
    return after
async def async_check(values):
    assert [item async for item in values]
def lexical_before(values):
    item = 1
    assert [item for item in values]
    return item
def lexical_after(values):
    assert [item for item in values]
    after = 1
    item = 2
    return (after, item)
def captured_comprehension(values):
    assert [lambda: item for item in values]
def asserted_local_reference():
    assert later
    first = 1
    later = 2
    return (first, later)
def unused_assertion_binding():
    assert (unused := 1)
    used = 2
    return used
",
        "optimized_comprehension_locals.py",
        &LEVELS,
    );
}

#[test]
fn removed_assertion_comprehensions_preserve_module_and_class_cells() {
    assert_matches_cpython(
        r"
assert [lambda: item for item in values]
class C:
    assert [lambda: item for item in values]
",
        "optimized_comprehension_cells.py",
        &LEVELS,
    );
    assert_matches_cpython(
        r"
assert [lambda: _ for _ in values]
x: int = 1
class C:
    assert [lambda: _ for _ in values]
    def method(self):
        return __class__
",
        "optimized_comprehension_cell_order.py",
        &LEVELS,
    );
}

#[test]
fn comprehension_temporaries_follow_surviving_execution_order() {
    assert_matches_cpython(
        r"
assert [x for x in a]
[y for y in b]
[x for x in c]
class C:
    assert [x for x in a]
    [y for y in b]
    [x for x in c]
",
        "optimized_comprehension_order.py",
        &LEVELS,
    );
}

#[test]
fn stripped_docstrings_match_cpython_flags_constants_and_locations() {
    for (source, filename) in [
        ("\"module docstring\"\n", "optimized_module_docstring.py"),
        (
            "\"doc\"\n\"\\ud800\"\ndef f():\n    \"doc\"\n    \"\"\n    x=1\n",
            "optimized_surrogate_after_docstring.py",
        ),
        ("\"doc\"\nx: int = 1\n", "optimized_docstring_annotation.py"),
        (
            "\"doc\"\nfrom __future__ import annotations\nx: int = 1\n",
            "optimized_docstring_future_annotation.py",
        ),
        (
            r#""module docstring"
"a later string expression"
def function():
    "function docstring"
    "another string expression"
    return "value"
def empty():
    "only a docstring"
async def coroutine():
    "coroutine docstring"
    return 1
def generator():
    "generator docstring"
    yield 1
class C:
    "class docstring"
    def method(self):
        "method docstring"
        return __class__
    class Empty:
        "only a nested class docstring"
"#,
            "optimized_docstrings.py",
        ),
        (
            r#"def outer():
    "retained value"
    def inner():
        "retained value"
        return "retained value"
    return inner
"#,
            "optimized_docstring_identity.py",
        ),
    ] {
        assert_matches_cpython(source, filename, &LEVELS);
    }
}

#[test]
fn removed_assertions_do_not_create_docstrings() {
    assert_matches_cpython(
        r#"assert True
"not a module docstring"
def function():
    assert True
    "not a function docstring"
class C:
    assert True
    "not a class docstring"
"#,
        "optimized_non_docstrings.py",
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
fn optimized_assertions_skip_codegen_only_syntax_errors() {
    assert_matches_cpython(
        "assert (yield 1)\n",
        "optimized_module_yield.py",
        &[OptimizationLevel::One, OptimizationLevel::Two],
    );
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
fn optimized_conditional_joins_match_cpython() {
    for source in [
        "def f(a, b):\n    for x in a:\n        if not (a and b):\n            assert a\n",
        "def f(a, b):\n    for x in a:\n        for y in a:\n            if a or b:\n                assert a\n",
        "def f(xs, a, b, c):\n    for x in xs:\n        if x == a:\n            assert x\n        elif x == b or x == c:\n            assert x\n",
        "def f(xs, a, b):\n    for x in xs:\n        if x == a or x == b:\n            assert x\n",
        "def f(x, a, b, c):\n    if x == a:\n        assert x\n    elif x == b or x == c:\n        assert x\n",
        "def f(xs, a, b, c):\n    for x in xs:\n        if x == a:\n            work(x)\n        elif x == b and x == c:\n            work(x)\n        else:\n            assert x\n    else:\n        fallback()\n",
        "def f(xs, a, b, c, gate):\n    if gate:\n        for x in xs:\n            if x == a:\n                work(x)\n            elif x == b and x == c:\n                work(x)\n            else:\n                assert x\n    else:\n        fallback()\n",
        "def f(xs, a, b, c):\n    for x in xs:\n        if x == a:\n            pass\n        elif x == b or x == c:\n            pass\n",
        "def f(xs, a, b, c):\n    for x in xs:\n        if x == a:\n            assert x\n        elif x == b or x == c:\n            assert x\n        work(x)\n",
        "events = []\na = True\nb = False\nc = True\nif b:\n    assert a\nelif a or c:\n    assert a\n",
        "events = []\na = True\nb = False\nc = True\nif b:\n    events.append('first')\nelif a or c:\n    assert a\n",
    ] {
        assert_matches_cpython(source, "case.py", &LEVELS);
    }
}

#[test]
fn optimized_conditional_join_in_exception_group_matches_cpython() {
    for source in [
        "def f(xs, a, b):\n    try:\n        work()\n    except* ValueError:\n        for x in xs:\n            if a or b:\n                assert x\n            else:\n                assert x\n",
        "def f(xs, a, b):\n    try:\n        work()\n    except* ValueError as err:\n        for x in xs:\n            if a or b:\n                assert x\n            else:\n                assert x\n",
    ] {
        assert_matches_cpython(source, "case.py", &LEVELS[1..]);
    }
}

#[test]
fn optimized_nested_else_join_locations_match_cpython() {
    for source in [
        "def f(x, obj, flag, other, items):\n    if flag:\n        if x == 0:\n            obj.channel = 0\n        else:\n            assert False, \"unknown\"\n    elif other:\n        obj.x = 42\n    obj.result = 3\n",
        "def f(x, y, z, obj, flag, items, manager):\n    try:\n        obj.before = 0\n        if flag:\n            if not (x < y < z):\n                obj.channel = 1\n            else:\n                pass\n        elif y:\n            obj.other = 2\n        obj.after = 3\n    finally:\n        obj.cleaned = True\n",
        "def f(x, y, z, obj, flag, items, manager):\n    obj.before = 0\n    if flag:\n        if x < y < z and flag:\n            try:\n                obj.channel = 1\n            finally:\n                obj.done = True\n        else:\n            assert y\n    elif y:\n        obj.other = 2\n    obj.after = 3\n",
        "def f(x, y, z, obj, flag, items, manager):\n    obj.before = 0\n    if flag:\n        if probe(x) and (y or z):\n            obj.channel = 1\n        else:\n            assert y\n    elif y:\n        obj.other = 2\n    obj.after = 3\n",
    ] {
        assert_matches_cpython(source, "case.py", &LEVELS);
    }
}

#[test]
fn optimized_nested_else_with_declarations_matches_cpython() {
    for source in [
        "def f(flag, x, y, obj):\n    if flag:\n        if x:\n            obj.value = 1\n        elif y:\n            obj.value = 2\n        else:\n            global g\n            assert x\n    elif y:\n        obj.value = 3\n    obj.done = True\n",
        "def f(flag, x, y, obj):\n    if flag:\n        if x:\n            obj.value = 1\n        else:\n            assert x\n            global g\n    elif y:\n        obj.value = 2\n    obj.done = True\n",
        "def outer():\n    captured = None\n    def f(flag, x, y, obj):\n        if flag:\n            if x:\n                obj.value = 1\n            else:\n                nonlocal captured\n                assert x\n        elif y:\n            obj.value = 2\n        obj.done = True\n    return f\n",
        "def f(flag, x, y, obj):\n    if flag:\n        if x:\n            obj.value = 1\n        else:\n            global g\n            assert x\n            pass\n    elif y:\n        obj.value = 2\n    obj.done = True\n",
    ] {
        assert_matches_cpython(source, "case.py", &LEVELS);
    }
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
fn protected_manager_multiple_passes_match_cpython() {
    let versions = [PythonVersion::Py314, PythonVersion::Py315];
    for source in [
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            pass
            global marker
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            global marker
            pass
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager as value: pass; pass; global marker
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    with other:
        with manager as value: pass; pass; global marker
    return value
",
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            global marker
            pass
            global spare_marker
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            pass
            pass
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager:
            pass
            pass
            global marker
    finally:
        cleanup()
    return 42
",
        r"def f(manager, other):
    try:
        with manager: pass; pass; global marker
    finally:
        cleanup()
    return 42
",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS, &versions);
    }
    for source in [
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            assert predicate()
            pass
            global marker
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager:
            pass
            assert predicate()
            pass
            global marker
    finally:
        cleanup()
    return 42
",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS[1..], &versions);
    }
}

#[test]
fn protected_manager_multiple_passes_with_nonlocal_match_cpython() {
    let versions = [PythonVersion::Py314, PythonVersion::Py315];
    for source in [
        r"def outer(manager, other):
    marker = 0
    def inner():
        try:
            with manager:
                pass
                pass
                nonlocal marker
        finally:
            cleanup()
    return inner
",
        r"def outer(manager, other):
    marker = 0
    def inner():
        try:
            with manager as value:
                pass
                nonlocal marker
                pass
        finally:
            cleanup()
    return inner
",
        r"def outer(manager, other):
    marker = 0
    def inner():
        try:
            with manager: pass; pass; nonlocal marker
        finally:
            cleanup()
    return inner
",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS, &versions);
    }
}

#[test]
fn manager_pass_before_declarations_keeps_tail_unprotected() {
    let versions = [PythonVersion::Py314, PythonVersion::Py315];
    for source in [
        r"def f(manager, other):
    with manager as value:
        global marker
        pass
        global spare_marker
    return value
",
        r"def f(manager, other):
    with manager as value:
        pass
        pass
        global marker
    return value
",
        r"def f(manager, other):
    with other:
        with manager as value:
            pass
            pass
            global marker
    return value
",
        r"def outer():
    captured = 1
    def f(manager, other):
        with manager as value:
            pass
            pass
            nonlocal captured
        return value
    return f
",
        r"def f(manager, other):
    with manager:
        global marker
        pass
        global spare_marker
    return 42
",
        r"def f(manager, other):
    with manager:
        pass
        pass
        global marker
    return 42
",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS, &versions);
    }
}

#[test]
fn legacy_manager_declaration_only_matches_cpython() {
    let versions = [PythonVersion::Py312, PythonVersion::Py313];
    for source in [
        r"def f(manager):
    with manager as value:
        global marker
    return value
",
        r"def f(manager):
    with manager:
        global marker
        global spare
    return 7
",
        r"def outer(manager):
    marker = 1
    def inner():
        with manager as value:
            nonlocal marker
        return value
    return inner
",
        r"async def f(manager):
    async with manager as value:
        global marker
    return value
",
        r"def f(manager):
    try:
        with manager:
            global marker
    finally:
        cleanup()
",
        r"def f(manager):
    with manager as value:
        assert predicate()
        global marker
    return value
",
        r"def f(manager):
    with manager as value:
        global marker
        assert predicate()
    return value
",
        r"def f(manager):
    try:
        with manager as value:
            global marker
            pass
    finally:
        cleanup()
    return value
",
        r"def f(manager):
    with manager as value:
        pass
    return value
",
        r"def f(manager):
    with manager as value: global marker
    return value
",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS, &versions);
    }
}

#[test]
fn legacy_protected_manager_pass_before_assertion_matches_cpython() {
    let versions = [PythonVersion::Py312, PythonVersion::Py313];
    for source in [
        r"def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            with inner:
                pass
                assert predicate()
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            with inner as value:
                pass
                global spare
                assert predicate()
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            with inner:
                pass
                pass
                nonlocal outer_marker
                assert True
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            with inner as obj.value:
                pass
                assert False
        except Exception:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            with outer:
                with inner:
                    assert predicate()
                    pass
                    assert predicate()
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    async def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            async with outer:
                with inner:
                    pass
                    assert predicate()
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        try:
            with inner as value: pass; assert predicate()
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(outer, inner, obj, predicate, cleanup, flag):
        with inner:
            pass
            assert predicate()
        return
    return f
",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS, &versions);
    }
}

#[test]
fn legacy_protected_manager_pass_before_work_matches_cpython() {
    let versions = [PythonVersion::Py312, PythonVersion::Py313];
    for source in [
        r"def f(manager, flag, items):
    try:
        with manager:
            pass
            raise ValueError()
    except Exception:
        recover()
",
        r"def f(manager, flag, items):
    try:
        with manager:
            pass
            work()
    finally:
        cleanup()
",
        r"def f(manager, flag, items):
    try:
        try:
            with manager:
                pass
                raise ValueError()
        finally:
            pass
    except Exception:
        recover()
    yield 1
",
        r"def f(manager, flag, items):
    try:
        with manager:
            pass; pass
            raise ValueError()
    finally:
        cleanup()
",
        r"def f(manager, flag, items):
    try:
        with manager, manager:
            pass
            match flag:
                case 1:
                    work()
                case _:
                    other()
    except Exception:
        recover()
",
        r"def f(manager, flag, items):
    try:
        with manager as value:
            pass
            assert flag
            work()
    finally:
        cleanup()
",
    ] {
        assert_matches_cpython_versions(source, "manager_pass.py", &LEVELS, &versions);
    }
}

#[test]
fn legacy_protected_manager_pass_before_split_try_matches_cpython() {
    let versions = [PythonVersion::Py312, PythonVersion::Py313];
    for source in [
        r"def f(manager):
    try:
        with manager:
            pass
            try:
                work()
            finally:
                cleanup()
    except Exception:
        recover()
",
        r"def f(manager, other):
    try:
        with manager, other:
            pass; pass
            global marker
            try:
                work()
            except Exception:
                recover()
            finally:
                cleanup()
    finally:
        finish()
",
        r"def f(manager):
    try:
        with manager:
            pass
            try:
                yield work()
            finally:
                cleanup()
    except Exception:
        recover()
",
        r"def f(manager):
    with manager:
        pass
        try:
            work()
        finally:
            cleanup()
",
        r"def f(manager):
    try:
        return work()
    finally:
        with manager:
            pass
            try:
                work()
            finally:
                cleanup()
",
        r"def f(manager):
    try:
        with manager:
            pass
            try:
                return work()
            finally:
                cleanup()
    except Exception:
        recover()
",
    ] {
        assert_matches_cpython_versions(source, "manager_split_pass.py", &LEVELS, &versions);
    }
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
                let native = oracle(
                    source.as_bytes(),
                    "modern_manager_finally.py",
                    python_version,
                    optimization_level,
                );
                assert!(
                    native.status.success(),
                    "{}",
                    String::from_utf8_lossy(&native.stderr)
                );
                let actual = compile_with_options(
                    source,
                    "modern_manager_finally.py",
                    &CompileOptions {
                        python_version,
                        optimization_level,
                        ..CompileOptions::default()
                    },
                )
                .unwrap()
                .marshal();
                for code in [&native.stdout, &actual] {
                    let result = run_with_input(
                        Command::new(ORACLES[index].get().unwrap())
                            .args(["-I", "-S", "-B", "-c", TRACE, mode]),
                        code,
                    );
                    assert!(
                        result.status.success(),
                        "{python_version}, {optimization_level:?}, {mode}: {}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                }
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
            let native = oracle(
                SOURCE.as_bytes(),
                "legacy_manager_finally.py",
                python_version,
                optimization_level,
            );
            assert!(
                native.status.success(),
                "{}",
                String::from_utf8_lossy(&native.stderr)
            );
            let actual = compile_with_options(
                SOURCE,
                "legacy_manager_finally.py",
                &CompileOptions {
                    python_version,
                    optimization_level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            assert_eq!(
                actual, native.stdout,
                "{python_version}, {optimization_level:?}"
            );
            for code in [&native.stdout, &actual] {
                let result = run_with_input(
                    Command::new(ORACLES[index].get().unwrap())
                        .args(["-I", "-S", "-B", "-c", TRACE]),
                    code,
                );
                assert!(
                    result.status.success(),
                    "{python_version}, {optimization_level:?}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
        }
    }
}

#[test]
fn legacy_protected_manager_pass_before_declaration_matches_cpython() {
    let versions = [PythonVersion::Py312, PythonVersion::Py313];
    for source in [
        r"def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        try:
            with manager as value:
                pass
                global marker
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        try:
            with manager:
                pass
                nonlocal outer_marker
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        try:
            with outer:
                with manager:
                    global spare
                    pass
                    global marker
        except Exception:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    async def f(manager, outer, obj, predicate, cleanup):
        try:
            async with outer:
                with manager as value:
                    pass
                    global marker
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        with manager as value:
            pass
            assert predicate()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        try:
            with manager: pass; assert predicate()
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    async def f(manager, outer, obj, predicate, cleanup):
        try:
            async with manager as value:
                pass
                global marker
        finally:
            cleanup()
        return
    return f
",
        r"def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        try:
            with manager:
                pass
                pass
                global marker
        except Exception:
            cleanup()
        return
    return f
",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS, &versions);
    }
    assert_matches_cpython_versions(
        r"def outer():
    outer_marker = 0
    def f(manager, outer, obj, predicate, cleanup):
        try:
            with manager:
                pass
                assert predicate()
        finally:
            cleanup()
        return
    return f
",
        "case.py",
        &LEVELS[1..],
        &versions,
    );
}

#[test]
fn legacy_protected_manager_multiple_passes_match_cpython() {
    let versions = [PythonVersion::Py312, PythonVersion::Py313];
    for source in [
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            pass
            global marker
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            global marker
            pass
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            pass
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager as value:
            pass
            global marker
            pass
            global spare_marker
    finally:
        cleanup()
    return value
",
        r"def f(manager, other):
    try:
        with manager:
            pass
            pass
            global marker
    finally:
        cleanup()
    return 42
",
        r"def outer(manager, other):
    marker = 0
    def inner():
        try:
            with manager as value:
                pass
                pass
                nonlocal marker
        finally:
            cleanup()
    return inner
",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS, &versions);
    }
}

#[test]
fn modern_manager_pass_before_declarations_matches_cpython() {
    let versions = [PythonVersion::Py314, PythonVersion::Py315];
    for source in [
        "def f(manager):\n    try:\n        with manager as value:\n            pass\n            global marker\n    finally:\n        cleanup()\n",
        "def f(manager):\n    try:\n        with manager as value:\n            global marker\n            pass\n            global spare_marker\n    finally:\n        cleanup()\n",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS, &versions);
    }
    for source in [
        "def f(manager):\n    try:\n        with manager as value:\n            pass\n            assert predicate()\n            global marker\n    finally:\n        cleanup()\n",
        "def f(manager):\n    try:\n        with manager as value:\n            assert predicate()\n            pass\n            global marker\n    finally:\n        cleanup()\n",
    ] {
        assert_matches_cpython_versions(source, "case.py", &LEVELS[1..], &versions);
    }
}

#[test]
fn nested_continue_with_elided_assertion_matches_cpython() {
    for source in [
        "def f(xs, a, cm):\n    for x in xs:\n        if a:\n            if x:\n                continue\n            global g\n            assert x\n",
        "def outer():\n    g = None\n    def f(xs, a, cm):\n        for x in xs:\n            if a:\n                if x:\n                    continue\n                nonlocal g\n                assert x\n    return f\n",
        "def f(xs, a, b):\n    for x in xs:\n        if a:\n            if b:\n                continue\n            assert x\n",
        "def f(a, b):\n    while a:\n        if a:\n            if b:\n                continue\n            assert False\n",
        "def f(xs, a, b):\n    for x in xs:\n        if a:\n            if b:\n                continue\n            assert x\n    else:\n        cleanup()\n",
        "def f(xs, a, b):\n    for x in xs:\n        if a:\n            if b:\n                continue\n            pass\n",
        "def f(xs, a, b):\n    for x in xs:\n        if a:\n            if b:\n                continue\n            assert x\n            cleanup()\n",
        "def f(xs, a, b, cm):\n    with cm:\n        for x in xs:\n            if a:\n                if b:\n                    continue\n                assert x\n",
    ] {
        assert_matches_cpython(source, "case.py", &LEVELS);
    }
}

#[test]
fn elided_assertion_with_nested_finally_matches_cpython() {
    for source in [
        "def f(flag):\n    try:\n        operation()\n        assert flag\n    except ValueError:\n        raise\n    finally:\n        try:\n            cleanup()\n        except TypeError:\n            pass\n",
        "def f(flag):\n    try:\n        operation()\n    except ValueError:\n        raise\n    finally:\n        try:\n            cleanup()\n        except TypeError:\n            pass\n",
        "def f(flag):\n    try:\n        operation()\n        global declared\n        assert flag\n    except ValueError:\n        raise\n    finally:\n        try:\n            cleanup()\n        except TypeError:\n            pass\n",
        "def outer(flag):\n    declared = None\n    def f():\n        try:\n            operation()\n            nonlocal declared\n            assert flag\n        except ValueError:\n            raise\n        finally:\n            try:\n                cleanup()\n            except TypeError:\n                pass\n    return f\n",
        "def f():\n    try:\n        operation()\n        global g\n    except ValueError:\n        raise\n    finally:\n        try:\n            cleanup()\n        except TypeError:\n            pass\n",
        "def outer():\n    x = None\n    def f():\n        try:\n            operation()\n            nonlocal x\n        except ValueError:\n            raise\n        finally:\n            try:\n                cleanup()\n            except TypeError:\n                pass\n    return f\n",
    ] {
        assert_matches_cpython(source, "case.py", &LEVELS);
    }
}

#[test]
fn optimized_manager_tail_after_runtime_work_matches_cpython() {
    for source in [
        "def f(cm):\n    with cm:\n        try:\n            action()\n        finally:\n            cleanup()\n        pass\n        assert value\n",
        "def f(cm):\n    with cm:\n        token = action()\n        try:\n            action()\n        finally:\n            cleanup()\n        assert token\n",
        "def f(cm):\n    with cm:\n        action()\n        try:\n            action()\n        finally:\n            cleanup()\n        assert value\n",
        "def f(cm):\n    with cm:\n        token = action()\n        pass\n        assert token\n",
    ] {
        assert_matches_cpython(source, "manager_runtime_tail.py", &LEVELS);
    }
}

#[test]
fn elided_assertion_after_manager_finally_matches_cpython() {
    for source in [
        "def f(cm, x):\n    with cm:\n        try:\n            work()\n        finally:\n            cleanup()\n        assert x\n",
        "def f(cm, x, y):\n    with cm:\n        try:\n            work()\n        finally:\n            cleanup()\n        assert x; assert y\n",
        "def f(cm, x):\n    with cm:\n        try:\n            work()\n        finally:\n            cleanup()\n        assert (value := x)\n    return value\n",
        "def f(cm, other, x):\n    with cm, other:\n        try:\n            work()\n        finally:\n            cleanup()\n        assert x\n",
        "def f(cm, x):\n    with cm:\n        try:\n            work()\n        finally:\n            cleanup()\n        assert x\n        after()\n",
        "async def f(cm, x):\n    await before()\n    with cm:\n        try:\n            work()\n        finally:\n            cleanup()\n        assert x\n",
        "def f(cm, x, y, value):\n    with cm:\n        try:\n            if x:\n                if y:\n                    work()\n                assert value\n            else:\n                other()\n        finally:\n            cleanup()\n        assert value\n",
        "def outer():\n    cell = None\n    def f(cm, x, y, value):\n        with cm:\n            try:\n                if x:\n                    if y:\n                        work()\n                    nonlocal cell\n                else:\n                    other()\n            finally:\n                cleanup()\n            assert value\n    return f\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "manager_finally_assert.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
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
            let native = oracle(
                SOURCE.as_bytes(),
                "manager_finally_assert.py",
                python_version,
                *optimization_level,
            );
            assert!(
                native.status.success(),
                "{}",
                String::from_utf8_lossy(&native.stderr)
            );
            let actual = compile_with_options(
                SOURCE,
                "manager_finally_assert.py",
                &CompileOptions {
                    python_version,
                    optimization_level: *optimization_level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            let mut outputs = Vec::new();
            for payload in [&native.stdout, &actual] {
                let result = run_with_input(
                    Command::new(ORACLES[index].get().unwrap())
                        .args(["-I", "-S", "-B", "-c", TRACE])
                        .stdout(Stdio::piped()),
                    payload,
                );
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                outputs.push(result.stdout);
            }
            assert_eq!(
                outputs[0], outputs[1],
                "{python_version}, {optimization_level:?}"
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
fn empty_try_before_finalizer_work_keeps_normal_exit_owner() {
    const SOURCE: &str = "async def f(manager, items, work, flag):\n    async with manager:\n        for item in items:\n            try:\n                await work(item)\n            finally:\n                if flag:\n                    value = work()\n                    count = value / 2\n                    try:\n                        assert count == item\n                    except AssertionError:\n                        pass\n                    work(count)\n";
    assert_matches_cpython(SOURCE, "empty_try_finally_work.py", &LEVELS[1..]);
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
                let native = oracle(
                    source.as_bytes(),
                    "case.py",
                    python_version,
                    optimization_level,
                );
                assert!(
                    native.status.success(),
                    "{}",
                    String::from_utf8_lossy(&native.stderr)
                );
                let options = CompileOptions {
                    python_version,
                    optimization_level,
                    ..CompileOptions::default()
                };
                let actual = compile_with_options(source, "case.py", &options)
                    .unwrap()
                    .marshal();
                let execute = |bytes: &[u8]| {
                    run_with_input(
                        Command::new(ORACLES[index].get().unwrap())
                            .args(["-I", "-S", "-c", TRACE_BACKEDGE])
                            .stdout(Stdio::piped()),
                        bytes,
                    )
                };
                let expected_run = execute(&native.stdout);
                let actual_run = execute(&actual);
                for result in [&expected_run, &actual_run] {
                    assert!(
                        result.status.success(),
                        "{}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                }
                assert_eq!(
                    expected_run.stdout, expected_events,
                    "{python_version}, {optimization_level:?}"
                );
                assert_eq!(
                    actual_run.stdout, expected_run.stdout,
                    "{python_version}, {optimization_level:?}"
                );
                assert_eq!(
                    actual, native.stdout,
                    "{python_version}, {optimization_level:?}"
                );
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
fn elided_assertions_preserve_async_comprehension_return_location() {
    for source in [
        r"async def f(xs):
    result = [item async for item in xs]
    assert result
",
        r"async def f(xs):
    result = [
        item
        async for item in xs
    ]
    assert result
    assert len(result)
",
        r"async def f(xs):
    result = [item async for item in xs]
    assert result
    pass
",
        r"async def f(xs):
    assert xs
    result = [item async for item in xs]
",
        r"async def f(xs):
    result = [item for item in xs]
    assert result
",
        r"async def f(xs):
    result = await collect(xs)
    assert result
",
    ] {
        assert_matches_cpython(source, "implicit_return.py", &LEVELS);
    }
}

#[test]
fn elided_assertions_preserve_async_set_and_dict_return_locations() {
    for source in [
        r"async def f(xs):
    result = {item async for item in xs}
    assert result
",
        r"async def f(xs):
    result = {item: item async for item in xs}
    assert result
",
    ] {
        assert_matches_cpython(source, "implicit_return_comp.py", &LEVELS);
    }
}

#[test]
fn elided_assertions_preserve_sync_comprehension_return_locations() {
    for source in [
        r"def f(xs):
    result = [
        item
        for item in xs
        if item
    ]
    assert result
",
        r"def f(xs):
    result = {
        item: item
        for item in xs
    }
    assert result
",
        r"def f(xs):
    result = {
        item
        for item in xs
    }
    assert result
",
        r"def f(xs):
    result = [item for item in xs]
    assert result
",
    ] {
        assert_matches_cpython(source, "implicit_return_sync.py", &LEVELS);
    }
}

#[test]
fn elided_assertions_preserve_nested_generator_return_locations() {
    for source in [
        r"def f(xs):
    result = [x for x in xs if any(test(x, y) for y in xs)]
    assert result
",
        r"def f(xs):
    result = {
        x: sum(test(x, y) for y in xs)
        for x in xs
    }
    assert result
",
        r"def f(xs):
    result = [x for x in xs if any(test(y) for y in xs)]
    assert result
",
        r"def f(xs):
    result = [x for x in xs if any(test(x, y) for y in xs)]
    assert result
    pass
",
    ] {
        assert_matches_cpython(source, "implicit_return_nested_gen.py", &LEVELS);
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
                let native = oracle(
                    source.as_bytes(),
                    "case.py",
                    python_version,
                    optimization_level,
                );
                assert!(
                    native.status.success(),
                    "{}",
                    String::from_utf8_lossy(&native.stderr)
                );
                let options = CompileOptions {
                    python_version,
                    optimization_level,
                    ..CompileOptions::default()
                };
                let actual = compile_with_options(&source, "case.py", &options)
                    .unwrap()
                    .marshal();
                assert_eq!(
                    actual, native.stdout,
                    "{python_version}, {optimization_level:?}, {expression}"
                );
                if matches!(python_version, PythonVersion::Py314 | PythonVersion::Py315) {
                    let expected = if protected_314 && python_version == PythonVersion::Py314 {
                        &b"completed\n"[..]
                    } else {
                        &b"escaped\n"[..]
                    };
                    for code in [&native.stdout, &actual] {
                        let result = run_with_input(
                            Command::new(ORACLES[index].get().unwrap())
                                .args(["-I", "-S", "-B", "-c", TRACE_CONDITION])
                                .arg(condition_line.to_string())
                                .stdout(Stdio::piped()),
                            code,
                        );
                        assert!(
                            result.status.success(),
                            "{}",
                            String::from_utf8_lossy(&result.stderr)
                        );
                        assert_eq!(
                            result.stdout, expected,
                            "{python_version}, {optimization_level:?}, {expression}"
                        );
                    }
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
                let executable = ORACLES[index].get().expect("oracle identity was checked");
                let level_arg = if level == OptimizationLevel::One {
                    "1"
                } else {
                    "2"
                };
                let mut child = Command::new(executable)
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
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(source.as_bytes())
                    .unwrap();
                let expected_text = child.wait_with_output().unwrap();
                assert!(
                    expected_text.status.success(),
                    "{}",
                    String::from_utf8_lossy(&expected_text.stderr)
                );
                let actual_text = compile_with_options(&source, "case.py", &options)
                    .unwrap()
                    .marshal();
                assert_eq!(
                    actual_text, expected_text.stdout,
                    "{version}, {level:?}, text"
                );

                for (label, payload) in [
                    ("native", expected.stdout.as_slice()),
                    ("serc", actual.as_slice()),
                ] {
                    let mut child = Command::new(executable)
                        .args([
                            "-I",
                            "-S",
                            "-X",
                            "utf8",
                            "-c",
                            IDENTITY_ORACLE,
                            if shared { "shared" } else { "distinct" },
                        ])
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                        .unwrap();
                    child.stdin.take().unwrap().write_all(payload).unwrap();
                    let identity = child.wait_with_output().unwrap();
                    assert!(
                        identity.status.success(),
                        "{version}, {level:?}, {label}: {}",
                        String::from_utf8_lossy(&identity.stderr)
                    );
                }
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
fn elided_assertions_after_conditional_comprehension_keep_return_locations() {
    for source in [
        "def f(x, y):\n    a = [v for v in x] if y else []\n    assert x\n",
        "def f(x, y):\n    a = {v for v in x} if y else y\n    assert x\n    assert y\n",
        "def f(x, y):\n    a = {v: v for v in x} if check(y) else []\n    assert x\n    assert y\n",
        "def f(x, y):\n    a = [v for v in x if v] if y else []\n    assert x\n",
        "def f(x, y):\n    a = [v + w for v in x for w in x] if y else []\n    assert x\n    assert y\n",
        "def f(x, y):\n    a = [(z := v) for v in x] if y else []\n    assert x\n",
        "def f(x, y):\n    a = [v for v in x] if y else [str(y)]\n    assert x\n",
        "def f(x, y):\n    a = [v for v in x] if y else []\n    assert (z := y)\n",
        "def f(x, y):\n    a = [(z := v) for v in x] if y else []\n    assert (z := y)\n",
        "def outer():\n    a = z = w = None\n    def f(x, y):\n        nonlocal a, z, w\n        a = [v for v in x] if y else []\n        assert (z := y)\n    return f\n",
        "def f(x, y, callback, manager):\n    with manager:\n        if x:\n            assert x\n        else:\n            assert y\n        callback(x)\n    a = [v for v in x] if y else []\n    assert x\n",
        "def f(x, y):\n    global a, z\n    a = [v for v in x] if y else []\n    assert x\n    assert y\n",
        "def f(x, y):\n    def inner():\n        return a, z\n    a = [(z := v) for v in x] if y else []\n    assert x\n    assert y\n",
        "def outer():\n    a = z = None\n    def f(x, y):\n        nonlocal a, z\n        a = [(z := v) for v in x] if y else []\n        assert x\n        assert y\n    return f\n",
        "def f(x, y):\n    try:\n        a = [v for v in x] if y else []\n        assert x\n    finally:\n        clean()\n",
        "def f(x, y, obj):\n    obj.a = [v for v in x] if y else []\n    assert x\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/conditional_comprehension_assert.py",
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

#[test]
fn elided_assertions_after_terminal_comprehension_branches_match_cpython() {
    for source in [
        "def f(x, y, z, cm):\n    if x:\n        a = [i for i in y]\n    assert x\n",
        "def f(x, y, z, cm):\n    if x:\n        a = {i for i in y}\n    assert x\n",
        "def f(x, y, z, cm):\n    if x:\n        a = {i: i for i in y}\n    assert x\n",
        "def f(x, y, z, cm):\n    if x:\n        a = [(i, j) for i in y if i for j in y if j]\n    assert x\n",
        "def f(x, y, z, cm):\n    if x:\n        a = [(b := i) for i in y]\n    assert x\n",
        "def f(x, y, z, cm):\n    if x:\n        work()\n    else:\n        a = [i for i in y]\n    assert x\n",
        "def f(x, y, z, cm):\n    if x:\n        a = [i for i in y]\n    elif z:\n        a = {i for i in y}\n    else:\n        a = {i:i for i in y}\n    assert x\n",
        "def f(x, y, z, cm):\n    if x or z:\n        assert z\n    else:\n        a = [i for i in y]\n    assert x\n",
        "def f(x, y, z, cm):\n    if x:\n        a = [i for i in y]\n    else:\n        return x\n    assert x\n",
        "def f(x, y, z, cm):\n    if x:\n        a = [i for i in y]\n    else:\n        raise ValueError()\n    assert x\n",
        "def f(x, y):\n    global a\n    if x:\n        a = [i for i in y]\n    assert x\n",
        "def outer():\n    a = None\n    def f(x, y):\n        nonlocal a\n        if x:\n            a = [i for i in y]\n        assert x\n    return f\n",
        "def f(x, y, z, cm):\n    if x:\n        a, b = [i for i in y]\n    assert x\n",
        "def f(x, y, z, cm):\n    if 1 < x < z:\n        a = [i for i in y]\n    else:\n        pass\n    assert x\n",
        "def f(x, y, z, cm):\n    try:\n        if x:\n            a = [i for i in y]\n        assert x\n    finally:\n        work()\n",
        "def f(x, y, z, cm):\n    with cm:\n        if x:\n            a = [i for i in y]\n        assert x\n",
    ] {
        assert_matches_cpython(source, "sub/terminal_comp_assert.py", &LEVELS);
    }
}

#[test]
fn comprehension_branches_preserve_backedge_store_locations() {
    let source = "def check_groups(groups, flag):\n    for group in groups:\n        if flag:\n            selected = [item for item in group if ready(item)]\n            assert selected\n        else:\n            selected = [item for item in group if item.active]\n            assert selected\n        assert len(selected) == 1\n";
    for source in [
        source.to_owned(),
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
fn comprehension_branches_preserve_sibling_return_donors() {
    for source in [
        "def f(items, flag, other):\n    if flag:\n        result = [item for item in items]\n        assert result\n    else:\n        if other:\n            if flag:\n                work()\n        assert flag\n    assert flag\n",
        "async def f(items, flag, other):\n    if flag:\n        result = [item for item in items]\n    else:\n        if other:\n            work()\n        else:\n            more()\n        assert flag\n    assert flag\n",
        "async def f(items, flag):\n    if flag:\n        result = [item for item in items]\n    else:\n        global token\n    assert flag\n",
    ] {
        assert_matches_cpython(source, "comprehension_sibling_return.py", &LEVELS);
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
fn elided_assertions_preserve_terminal_branch_comprehension_returns() {
    for source in [
        r"def f(flag, xs):
    if flag:
        result = [x for x in xs]
        assert result
",
        r"async def f(flag, xs):
    if flag:
        result = [x for x in xs]
        assert result
",
        r"def f(flag, xs):
    if flag:
        result = {x for x in xs}
        assert result
",
        r"def f(flag, xs):
    if flag:
        result = {x: x for x in xs}
        assert result
",
        r"def f(flag, xs):
    if flag:
        result = [x for x in xs]
        assert result
    else:
        assert flag
",
        r"def f(flag, xs):
    if flag:
        assert flag
    else:
        result = [x for x in xs]
        assert result
",
        r"def f(flag, xs):
    if flag:
        result = [x for x in xs]
        assert result
        pass
",
        r"def f(flag, xs):
    try:
        if flag:
            result = [x for x in xs]
            assert result
    except LookupError:
        recover()
",
        r"def f(flag, xs):
    try:
        if flag:
            result = [x for x in xs]
            assert result
    finally:
        cleanup()
",
        r"def f(flag, xs, manager):
    with manager:
        if flag:
            result = [x for x in xs]
            assert result
",
    ] {
        assert_matches_cpython(source, "terminal_comprehension_assert.py", &LEVELS);
    }
}

#[test]
fn nested_conditional_except_before_finally_matches_cpython() {
    for source in [
        "def f(x, d):\n    try:\n        try:\n            if x:\n                pass\n        except Exception:\n            pass\n    finally:\n        d.clear()\n",
        "async def f(x, d):\n    try:\n        try:\n            if x:\n                work()\n        except:\n            pass\n    finally:\n        d.clear()\n",
        "def f(x, d):\n    try:\n        try:\n            if x:\n                pass\n        except Exception:\n            pass\n        else:\n            work()\n    finally:\n        d.clear()\n",
        "def f(x, d):\n    try:\n        if x:\n            pass\n    except Exception:\n        pass\n    d.clear()\n",
    ] {
        assert_matches_cpython(source, "case.py", &LEVELS);
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
            for (label, payload) in [
                ("native", expected.stdout.as_slice()),
                ("serc", actual.as_slice()),
            ] {
                let result = run_with_input(
                    Command::new(ORACLES[index].get().unwrap())
                        .args(["-I", "-S", "-X", "utf8", "-c", TRACE_ORACLE])
                        .stdout(Stdio::piped()),
                    payload,
                );
                assert!(
                    result.status.success(),
                    "{version}, {level:?}, {label}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
        }
    }
}

#[test]
fn nested_chained_condition_in_generator_loop_tail_matches_cpython() {
    assert_matches_cpython_versions(
        "def test(xs, y, z):\n    yield 0\n    for x in xs:\n        if x:\n            if x < y < z:\n                yield x\n        else:\n            assert z\n",
        "tests/nested_generator_chain.py",
        &[OptimizationLevel::One, OptimizationLevel::Two],
        &[PythonVersion::Py314, PythonVersion::Py315],
    );
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
            for (label, payload) in [
                ("native", expected.stdout.as_slice()),
                ("serc", actual.as_slice()),
            ] {
                let result = run_with_input(
                    Command::new(ORACLES[index].get().unwrap())
                        .args(["-I", "-S", "-X", "utf8", "-c", TRACE_ORACLE])
                        .stdout(Stdio::piped()),
                    payload,
                );
                assert!(
                    result.status.success(),
                    "{version}, {level:?}, {label}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
        }
    }
}

#[test]
fn elided_assert_after_nested_while_keeps_backedge_location() {
    for source in [
        r"def f(xs,x,y,cm):
    while xs:
        if x:
            while y:
                if y:
                    raise ValueError
            assert x
        else:
            break
",
        r"def f(xs,x,y,cm):
    while xs:
        if x:
            while y:
                if y:
                    return 1
            assert x
        else:
            continue
",
        r"def f(xs,x,y,cm):
    while xs:
        if x:
            while y:
                if y:
                    work()
            assert x
        else:
            work()
",
        r"def f(xs,x,y,cm):
    for z in xs:
        if x:
            while y:
                if y:
                    work()
            assert x
        else:
            return 1
",
        r"def f(xs,x,y):
    while xs:
        if x:
            while y:
                if y:
                    raise ValueError
            assert x
            assert y
        else:
            break
",
        r"def f(xs,x,y):
    while xs:
        if x:
            while y:
                if y:
                    raise ValueError
        else:
            break
",
        r"def f(xs,x,y):
    while xs:
        if x:
            while y:
                if y:
                    raise ValueError
            assert x
            pass
        else:
            break
",
        r"def f(xs,x,y):
    while xs:
        if x:
            while y:
                if y:
                    raise ValueError
            else:
                work()
            assert x
        else:
            break
",
        r"def f(xs,x,y,cm):
    while xs:
        if x:
            while y:
                try:
                    work()
                finally:
                    clean()
            assert x
        else:
            break
",
        r"def f(xs,x,y,cm):
    while xs:
        if x:
            while y:
                with cm:
                    work()
            assert x
        else:
            break
",
        r"def f(xs,x,y):
    while xs:
        if x:
            while (
                y()
            ):
                if x:
                    raise ValueError
            assert x
        else:
            break
",
        r"def f(xs,x,y,z):
    while xs:
        if x:
            while y:
                if x:
                    raise ValueError
            assert x
        elif z:
            work()
        else:
            break
",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/nested_while_elided.py",
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
            let native = oracle(
                source.as_bytes(),
                "fallthrough_nested_try.py",
                PythonVersion::Py314,
                optimization_level,
            );
            assert!(
                native.status.success(),
                "{}",
                String::from_utf8_lossy(&native.stderr)
            );
            let actual = compile_with_options(
                source,
                "fallthrough_nested_try.py",
                &CompileOptions {
                    python_version: PythonVersion::Py314,
                    optimization_level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            for code in [&native.stdout, &actual] {
                let result = run_with_input(
                    Command::new(ORACLES[2].get().unwrap())
                        .args(["-I", "-S", "-B", "-c", TRACE_CONDITION])
                        .arg(line.to_string())
                        .stdout(Stdio::piped()),
                    code,
                );
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(
                    result.stdout, expected,
                    "{optimization_level:?}, line {line}"
                );
            }
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
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ],
        );
    }
}

#[test]
fn elided_match_case_assertions_keep_success_locations() {
    for source in [
        r"def f(x, y):
    match x:
        case False:
            work()
        case str():
            assert x
        case True:
            work()
",
        r"def f(x, y):
    match x:
        case False:
            work()
        case str(v):
            assert x
            assert y
        case True:
            work()
",
        r"def f(x, y):
    match x:
        case [1] | [2]:
            assert x
        case False:
            work()
        case True:
            work()
",
        r"def f(x, y):
    match x:
        case False:
            work()
        case 1:
            assert x
        case True:
            work()
",
        r"def f(x, y):
    match x:
        case False:
            work()
        case str():
            assert x
            pass
        case True:
            work()
",
        r"def f(x, y):
    match x:
        case False:
            work()
        case str() if y:
            assert (v := x)
        case True:
            work()
",
        r"def f(x, y):
    global v
    match x:
        case False:
            work()
        case str():
            assert (v := x)
            assert y
        case True:
            work()
",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/elided_match.py",
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

#[test]
fn optimized_generator_assertion_joins_match_cpython() {
    for source in [
        "async def f(x, y):\n    if x:\n        assert x\n    else:\n        assert y\n    await work()\n",
        "async def f(x, y, manager):\n    async with manager:\n        if x:\n            assert x\n        else:\n            assert y\n        await work()\n",
        "def f(x, y):\n    if x and y:\n        assert x\n    else:\n        assert y\n    work()\n    yield 1\n",
        "async def f(x, y, manager, other):\n    async with manager:\n        with other:\n            if x:\n                assert x\n            else:\n                assert y\n            await work()\n    yield 1\n",
        "async def f(x, y, manager):\n    async with manager:\n        if x: assert x\n        else: assert y\n        work()\n",
        "async def f(x, y, manager):\n    async with manager:\n        if x or y:\n            assert x\n        else:\n            assert y\n        work()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "optimized_generator_join.py",
            &LEVELS,
            &[PythonVersion::Py314],
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
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ],
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
            let native = oracle(
                SOURCE.as_bytes(),
                "nested_pass_exit.py",
                version,
                optimization_level,
            );
            assert!(native.status.success());
            let actual = compile_with_options(
                SOURCE,
                "nested_pass_exit.py",
                &CompileOptions {
                    python_version: version,
                    optimization_level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            assert_eq!(actual, native.stdout, "{version} {optimization_level:?}");
            for payload in [&native.stdout, &actual] {
                let result = run_with_input(
                    Command::new(ORACLES[index].get().unwrap()).args(["-I", "-S", "-c", TRACE]),
                    payload,
                );
                assert!(
                    result.status.success(),
                    "{version} {optimization_level:?}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
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
        let native = oracle(
            SOURCE.as_bytes(),
            "optimized_with_join_owner.py",
            PythonVersion::Py314,
            level,
        );
        assert!(native.status.success());
        let actual = compile_with_options(
            SOURCE,
            "optimized_with_join_owner.py",
            &CompileOptions {
                python_version: PythonVersion::Py314,
                optimization_level: level,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .marshal();
        assert_eq!(actual, native.stdout);
        let index = VERSIONS
            .iter()
            .position(|(version, _, _)| *version == PythonVersion::Py314)
            .unwrap();
        for payload in [&native.stdout, &actual] {
            let result = run_with_input(
                Command::new(ORACLES[index].get().unwrap()).args(["-I", "-S", "-c", TRACE]),
                payload,
            );
            assert!(
                result.status.success(),
                "{level:?}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
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
            let native = oracle(SOURCE.as_bytes(), "protected_with_join.py", version, level);
            assert!(native.status.success());
            let actual = compile_with_options(
                SOURCE,
                "protected_with_join.py",
                &CompileOptions {
                    python_version: version,
                    optimization_level: level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            assert_eq!(actual, native.stdout);
            let index = VERSIONS
                .iter()
                .position(|(candidate, _, _)| *candidate == version)
                .unwrap();
            for payload in [&native.stdout, &actual] {
                let result = run_with_input(
                    Command::new(ORACLES[index].get().unwrap()).args(["-I", "-S", "-c", TRACE]),
                    payload,
                );
                assert!(
                    result.status.success(),
                    "{version:?} {level:?}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
        }
    }
}

#[test]
fn protected_with_assertion_join_before_pass_matches_cpython() {
    assert_matches_cpython_versions(
        "def f(flag, manager):\n    try:\n        with manager:\n            if flag:\n                assert flag\n            else:\n                assert not flag\n            pass\n        work()\n    except Exception:\n        recover()\n",
        "protected_with_join_pass.py",
        &LEVELS,
        &[PythonVersion::Py314, PythonVersion::Py315],
    );
    for source in [
        "def f(flag, other, manager):\n    try:\n        with manager:\n            if flag or other:\n                assert flag\n            else:\n                assert not flag\n            work()\n        work()\n    except Exception:\n        recover()\n",
        "def f(flag, manager):\n    try:\n        with manager:\n            if flag:\n                assert flag\n            else:\n                assert not flag\n            work()\n        work()\n    except Exception:\n        recover()\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "protected_with_join_pass.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn optimized_match_cases_inside_loops_match_cpython() {
    for source in [
        "def f(xs,b):\n    for x in xs:\n        match x:\n            case str():\n                assert x\n            case int():\n                work(x)\n            case _:\n                other()\n        assert x\n",
        "def f(xs,b):\n    for x in xs:\n        match x:\n            case str() | bytes():\n                assert (z := x)\n            case int():\n                work(x)\n            case _:\n                other()\n        z=x\n    else:\n        finished()\n",
        "def f(xs,b):\n    for group in xs:\n        for x in group:\n            match x:\n                case str() | bytes():\n                    assert x\n                case int():\n                    work(x)\n                case _:\n                    other()\n            z=[v for v in xs]\n",
        "def f(xs,b):\n    while xs:\n        x=xs.pop()\n        match x:\n            case str():\n                assert x\n            case int():\n                work(x)\n            case _:\n                other()\n        assert x\n",
        "def f(xs):\n    while xs:\n        x = xs.pop()\n        match x:\n            case str():\n                assert x\n            case _:\n                other()\n        pass\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "sub/elided_loop.py",
            &LEVELS[1..],
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
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
            let native = oracle(
                SOURCE.as_bytes(),
                "protected_with_join_pass.py",
                version,
                level,
            );
            assert!(native.status.success());
            let actual = compile_with_options(
                SOURCE,
                "protected_with_join_pass.py",
                &CompileOptions {
                    python_version: version,
                    optimization_level: level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            assert_eq!(actual, native.stdout);
            let index = VERSIONS
                .iter()
                .position(|(target, _, _)| *target == version)
                .unwrap();
            for payload in [&native.stdout, &actual] {
                let result = run_with_input(
                    Command::new(ORACLES[index].get().unwrap()).args(["-I", "-S", "-c", TRACE]),
                    payload,
                );
                assert!(
                    result.status.success(),
                    "{version:?} {level:?}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
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
fn optimized_assertions_after_nested_async_for_keep_manager_exit_location() {
    for source in [
        "async def f(outer, inner, items):\n    with outer:\n        with inner:\n            results: list[Item] = []\n            async for item in items():\n                results.append(item)\n            assert len(results) == 1\n            assert results[0].value\n",
        "async def f(first, second, third, items):\n    with first, second:\n        with third:\n            async for item in items:\n                work(item)\n            assert items\n",
    ] {
        assert_matches_cpython(source, "nested_async_for_manager_exit.py", &LEVELS);
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
        let native = oracle(
            SOURCE.as_bytes(),
            "async_for_assert.py",
            PythonVersion::Py314,
            level,
        );
        assert!(native.status.success());
        let actual = compile_with_options(
            SOURCE,
            "async_for_assert.py",
            &CompileOptions {
                python_version: PythonVersion::Py314,
                optimization_level: level,
                ..CompileOptions::default()
            },
        )
        .unwrap()
        .marshal();
        assert_eq!(actual, native.stdout);
        for payload in [&native.stdout, &actual] {
            let result = run_with_input(
                Command::new(ORACLES[2].get().unwrap()).args(["-I", "-S", "-c", TRACE]),
                payload,
            );
            assert!(
                result.status.success(),
                "{level:?}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
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
fn erased_loop_arms_keep_distinct_condition_backedges() {
    let source = r"def f(items, obj, enabled, active, alternate, left, right, offset, work):
    for item in items:
        value = item + offset
        if obj.flag or enabled:
            assert value
            if item is not None:
                assert value
                assert enabled
        else:
            assert value
            if item is not None:
                assert value
                assert enabled
        if active and (item is left or item is right):
            assert value
            assert enabled
        elif alternate and (item is left or item is right):
            assert value
            assert enabled
            assert active
            assert alternate
        else:
            assert value
            assert enabled
";
    assert_matches_cpython(source, "erased_loop_condition_backedges.py", &LEVELS);
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
