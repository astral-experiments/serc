#[allow(dead_code)]
mod common;

use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

fn assert_expression_cleanup_matches_cpython(source: &str, filename: &str) {
    for (version, variable, fallback) in [
        (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
        (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
        (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
        (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
    ] {
        let target = version.target();
        let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let oracle = "import importlib.util, marshal, sys; assert sys.implementation.name == 'cpython'; assert sys.version.split()[0] == sys.argv[1]; assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]; code = compile(sys.stdin.buffer.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
        let mut child = Command::new(python)
            .args(["-B", "-I", "-S", "-c", oracle])
            .args([
                target.tag.trim_start_matches('v'),
                &target.magic_hex(),
                filename,
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
        let expected = child.wait_with_output().unwrap();
        assert!(
            expected.status.success(),
            "{version:?}: {}",
            String::from_utf8_lossy(&expected.stderr)
        );
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for compiled in [
            compile_with_options(source, filename, &options).unwrap(),
            compile_bytes_with_options(source.as_bytes(), filename, &options).unwrap(),
        ] {
            assert!(
                compiled.marshal() == expected.stdout,
                "{version:?}:\n{source}"
            );
        }
    }
}

fn indent(source: &str) -> String {
    let mut indented = String::new();
    for line in source.lines() {
        indented.push_str("    ");
        indented.push_str(line);
        indented.push('\n');
    }
    indented
}

#[test]
fn global_call_compaction_retains_the_original_normal_finally_null_slot() {
    for asynchronous in [false, true] {
        for condition in ["value", "resource is not None", "resource.attr is None"] {
            for calls in 0..=4 {
                let definition = if asynchronous { "async def" } else { "def" };
                let manager = if asynchronous { "async with" } else { "with" };
                let prefix = "        record()\n".repeat(calls);
                let source = format!(
                    "{definition} sample(resource, flag):\n    try:\n        {manager} manager():\n            if flag:\n                item = resource\n{prefix}        value = factory(items={{resource: item}}, key=helper())\n        yield produce(value, extra=Owner.method(item))\n    finally:\n        if {condition}:\n            finish()\n"
                );
                assert_expression_cleanup_matches_cpython(&source, "global_finally_slot.py");
            }
        }
    }
}

#[test]
fn copied_named_handler_iterator_none_marker_reuses_its_cleanup_cell() {
    for condition in ["resource is None", "resource is not None"] {
        for enclosing_loop in [false, true] {
            let body = "for item in items:\n    try:\n        work(item)\n    except Error as error:\n        if flag:\n            record()\n            return\n";
            let body = if enclosing_loop {
                format!("while flag:\n{}", indent(body))
            } else {
                body.to_owned()
            };
            let source = format!(
                "async def sample(resource, items, flag):\n    try:\n{}    finally:\n        if {condition}:\n            await close(resource)\n",
                indent(&indent(&body))
            );
            assert_expression_cleanup_matches_cpython(&source, "copied_named_iterator_none.py");
        }
    }
}

#[test]
fn copied_saved_handler_manager_none_marker_reuses_its_cleanup_cell() {
    for condition in ["resource is None", "resource is not None"] {
        for returned in ["resource", "recover(resource, mode='retry')"] {
            for ordinary_try in [false, true] {
                let body = format!(
                    "with manager():\n    try:\n        work()\n    except Error:\n        if flag:\n            record()\n            return {returned}\n"
                );
                let body = if ordinary_try {
                    format!("try:\n{}except OtherError:\n    failed()\n", indent(&body))
                } else {
                    body
                };
                let source = format!(
                    "async def sample(resource, flag):\n    try:\n{}    finally:\n        if {condition}:\n            await close(resource)\n",
                    indent(&indent(&body))
                );
                assert_expression_cleanup_matches_cpython(
                    &source,
                    "copied_saved_handler_manager.py",
                );
            }
        }
    }
}

#[test]
fn copied_generator_handler_manager_none_marker_reuses_its_cleanup_cell() {
    for condition in ["resource is None", "resource is not None"] {
        let source = format!(
            "def sample(resource):\n    try:\n        while True:\n            with manager():\n                try:\n                    item = work()\n                except Error:\n                    return\n            yield item\n    finally:\n        if {condition}:\n            close(resource)\n"
        );
        assert_expression_cleanup_matches_cpython(&source, "copied_generator_handler_manager.py");
    }
}

#[test]
fn iterator_return_truth_marker_reuses_the_original_finalizer_setup() {
    for scope in ["def", "async def"] {
        for work in ["effect()", "effect(value)", "pass", "assert value"] {
            for cleanup in ["recover()", "recover(value)"] {
                let source = format!(
                    "{scope} sample(flag, combine, value, item, values):\n\
                     \x20   try:\n\
                     \x20       for item in values:\n\
                     \x20           result = combine(value if item else None, global_0(), global_1(), global_2())\n\
                     \x20           return result\n\
                     \x20   finally:\n\
                     \x20       try:\n\
                     \x20           if flag:\n\
                     \x20               {work}\n\
                     \x20       finally:\n\
                     \x20           {cleanup}\n"
                );
                assert_expression_cleanup_matches_cpython(&source, "retained_finalizer_setup.py");
            }
        }
    }
}

#[test]
fn folded_finally_iterable_preserves_the_original_operand_owner() {
    for iterable in ["(1,)", "(1, 2)", "(1, 2, 3)"] {
        for condition in ["flag", "flag == expected"] {
            for managers in ["manager", "outer, manager"] {
                let source = format!(
                    "def f(manager, outer, flag, expected):\n    yield None\n    with {managers}:\n        try:\n            assert {condition}\n        finally:\n            for value in {iterable}:\n                cleanup(value)\n",
                );
                assert_expression_cleanup_matches_cpython(&source, "folded_finally_iterable.py");
            }
        }
    }
    for condition in [
        "await check() == expected",
        "(await check()).result == expected",
        "len(await check()) == expected",
    ] {
        for managers in ["manager", "outer, manager"] {
            for cleanup in ["cleanup(value)", "await cleanup(value)"] {
                let source = format!(
                    "async def f(manager, outer, expected):\n    with {managers}:\n        try:\n            assert {condition}\n        finally:\n            for value in (1, 2, 3):\n                {cleanup}\n",
                );
                assert_expression_cleanup_matches_cpython(
                    &source,
                    "awaited_folded_finally_iterable.py",
                );
            }
        }
    }
}

#[test]
fn named_handler_if_elif_cleanup_is_copied_inside_finalizers() {
    for scope in ["def", "async def"] {
        for guard in ["other", "other is None", "other is not None"] {
            for then in ["value = True", "cleanup(error)"] {
                let source = format!(
                    "{scope} sample(flag, other, value, work, cleanup):\n\
                     \x20   try:\n\
                     \x20       work(value)\n\
                     \x20   finally:\n\
                     \x20       try:\n\
                     \x20           work(value)\n\
                     \x20       except Exception as error:\n\
                     \x20           if flag:\n\
                     \x20               {then}\n\
                     \x20           elif {guard}:\n\
                     \x20               value = error\n\
                     \x20   cleanup(value)\n\
                     \x20   return value\n"
                );
                assert_expression_cleanup_matches_cpython(&source, "named_finally_cleanup.py");
            }
        }
    }
}

#[test]
fn copied_finalizer_return_location_crosses_only_its_outer_try_pop() {
    for handler in ["none", "bare", "named"] {
        for guard in [false, true] {
            for cleanup in ["cleanup(value)", "await cleanup(value)"] {
                let returning = if guard {
                    "if flag:\n    return recover(value)\nreturn value\n"
                } else {
                    "return recover(value)\n"
                };
                let recovery = match handler {
                    "none" => returning.to_owned(),
                    "bare" => format!(
                        "try:\n    await work(value)\nexcept Error:\n{}",
                        indent(returning)
                    ),
                    "named" => format!(
                        "try:\n    await work(value)\nexcept Error as error:\n{}",
                        indent(returning)
                    ),
                    _ => unreachable!(),
                };
                let protected = format!("try:\n{}finally:\n    {cleanup}\n", indent(&recovery));
                let iteration = format!("for value in values:\n{}", indent(&protected));
                let body = format!(
                    "try:\n{}except Other as other:\n    recover(other)\n",
                    indent(&iteration)
                );
                for body in [body.clone(), format!("with manager:\n{}", indent(&body))] {
                    let source = format!(
                        "async def sample(values, value, flag, work, cleanup, recover, manager):\n{}",
                        indent(&body),
                    );
                    assert_expression_cleanup_matches_cpython(
                        &source,
                        "outer_try_iterator_return.py",
                    );
                }
            }
        }
    }
    for scope in ["def", "async def"] {
        for returning in [
            "return recover(value)\n",
            "if flag:\n    return recover(value)\n",
        ] {
            let recovery = format!(
                "try:\n    work(value)\nexcept Error:\n{}",
                indent(returning)
            );
            let protected = format!("try:\n{}finally:\n    cleanup(value)\n", indent(&recovery));
            let iteration = format!("for value in values:\n{}", indent(&protected));
            let body = format!(
                "try:\n{}except Other as other:\n    recover(other)\n",
                indent(&iteration)
            );
            let source = format!(
                "{scope} sample(values, value, flag, work, cleanup, recover):\n{}",
                indent(&body)
            );
            assert_expression_cleanup_matches_cpython(&source, "outer_try_iterator_return.py");
        }
    }
}

#[test]
fn terminal_raising_guards_reach_only_their_original_outer_finalizer_pop() {
    for (scope, work) in [
        ("def", "work(value)"),
        ("def", "yield value"),
        ("async def", "await work(value)"),
        ("async def", "yield value"),
    ] {
        for clause in ["else", "finally"] {
            for handler in [
                "except:\n    raise\n",
                "except Exception as caught:\n    raise error from caught\n",
            ] {
                for condition in ["flag", "error is not None"] {
                    let inner = format!(
                        "try:\n    {work}\n{handler}{clause}:\n    cleanup(value)\n    if {condition}:\n        raise error from None\n"
                    );
                    let outer = format!("try:\n{}finally:\n    cleanup(value)\n", indent(&inner));
                    let source = format!(
                        "{scope} sample(flag, error, value, work, cleanup):\n{}",
                        indent(&outer)
                    );
                    assert_expression_cleanup_matches_cpython(
                        &source,
                        "terminal_raising_guard_finalizer.py",
                    );
                }
            }
        }
    }
}

#[test]
fn completed_recovery_keeps_the_original_normal_finally_join_inside_its_manager() {
    let bodies = [
        "if flag:\n    if other:\n        cleanup()\n    else:\n        try:\n            recover()\n        except Error:\n            pass\n",
        "if flag:\n    try:\n        cleanup()\n    except Error:\n        pass\n",
        "if flag:\n    with inner:\n        cleanup()\n",
    ];
    for scope in ["def", "async def"] {
        for finalbody in bodies {
            let body = format!("try:\n    work()\nfinally:\n{}", indent(finalbody));
            let managed = format!("with manager:\n{}", indent(&body));
            for wrapped in [
                managed.clone(),
                format!("with outer:\n{}", indent(&managed)),
                format!("with outer, manager:\n{}", indent(&body)),
                format!("for value in values:\n{}", indent(&managed)),
                format!(
                    "for value in values:\n    for second in values:\n{}",
                    indent(&indent(&managed))
                ),
            ] {
                let source = format!(
                    "{scope} f(manager, outer, inner, flag, other, values):\n{}",
                    indent(&wrapped)
                );
                assert_expression_cleanup_matches_cpython(
                    &source,
                    "normal_finally_manager_join.py",
                );
            }
        }
    }
}

#[test]
fn completed_await_preserves_both_ordinary_try_pops_before_copied_finally() {
    for condition in ["value is None", "value is not None"] {
        for variant in [
            "nested",
            "combined",
            "ordinary_else",
            "awaited_handler",
            "named_handler",
            "loop_else",
            "awaited_loop",
        ] {
            let handler = match variant {
                "awaited_handler" => "except Error:\n    await work(value)\n",
                "named_handler" => "except Error as error:\n    recover(value)\n",
                _ => "except Error:\n    recover(value)\n",
            };
            let ordinary_else = if variant == "ordinary_else" {
                "else:\n    finish(value)\n"
            } else {
                ""
            };
            let inner = format!("try:\n    return await work(value)\n{handler}{ordinary_else}");
            let protected = if variant == "combined" {
                format!(
                    "try:\n{}except Other:\n    recover(value)\nfinally:\n    if {condition}:\n        finish(value)\n",
                    indent(&inner),
                )
            } else {
                let ordinary = format!(
                    "try:\n{}except Error:\n    recover(value)\n",
                    indent(&inner),
                );
                format!(
                    "try:\n{}finally:\n    if {condition}:\n        finish(value)\n",
                    indent(&ordinary),
                )
            };
            let test = if variant == "awaited_loop" {
                "await work(flag)"
            } else {
                "flag"
            };
            let loop_else = if variant == "loop_else" {
                "else:\n    finish(value)\n"
            } else {
                ""
            };
            let body = format!("while {test}:\n{}{loop_else}", indent(&protected));
            let source = format!("async def f(value, flag):\n{}", indent(&body));
            assert_expression_cleanup_matches_cpython(&source, "completed_await_ordinary_pops.py");
        }
    }
}

#[test]
fn completed_await_preserves_ordinary_pops_inside_the_unwound_while() {
    for condition in ["value is None", "value is not None"] {
        for manager in [false, true] {
            for ordinary_else in [false, true] {
                for returned in ["value", "await work(value)"] {
                    let innermost = if manager {
                        format!("async with manager:\n    return {returned}\n")
                    } else {
                        "try:\n    return await work(value)\nexcept InnerError:\n    recover(value)\n".to_string()
                    };
                    let otherwise = if ordinary_else {
                        "else:\n    finish(value)\n"
                    } else {
                        ""
                    };
                    let body = format!(
                        "try:\n{}except Error:\n    recover(value)\n{otherwise}",
                        indent(&innermost)
                    );
                    let protected = format!("while flag:\n{}", indent(&body));
                    let source = format!(
                        "async def f(value, flag, manager):\n    try:\n{}    except OuterError:\n        recover(value)\n    finally:\n        if {condition}:\n            finish(value)\n",
                        indent(&indent(&protected))
                    );
                    assert_expression_cleanup_matches_cpython(
                        &source,
                        "held_await_ordinary_pops.py",
                    );
                }
            }
        }
    }
}

#[test]
fn retargeted_manager_exit_donates_to_the_original_normal_finally_pop() {
    for condition in ["protocol is None", "protocol is not None"] {
        for handler in ["raise", "raise OtherError(error)"] {
            for (parameters, prefix, free) in [
                ("manager, protocol, registration", "", false),
                (
                    "manager, protocol, registration",
                    "keep = lambda: protocol\n",
                    false,
                ),
                ("manager, registration", "", true),
            ] {
                let protected = format!(
                    "{prefix}try:\n    async with manager:\n        raise Error\nexcept Error as error:\n    {handler}\nfinally:\n    try:\n        if {condition}:\n            await protocol.close()\n    finally:\n        await registration.revoke()\n",
                );
                for body in [
                    protected.clone(),
                    format!("while active:\n{}", indent(&protected)),
                ] {
                    let mut source = format!("async def f({parameters}):\n{}", indent(&body));
                    if free {
                        source = format!("def outer(protocol):\n{}    return f\n", indent(&source));
                    }
                    assert_expression_cleanup_matches_cpython(
                        &source,
                        "retargeted_manager_finally.py",
                    );
                }
            }
        }
    }
}

#[test]
fn awaited_normal_finally_keeps_pop_and_nested_setup_locations_distinct() {
    for terminal in ["raise Error(value)", "return value"] {
        for handler in [
            "except Error:\n    raise\n",
            "except Error as error:\n    failed = error\n    raise\n",
        ] {
            for guarded in [false, true] {
                let cleanup_handler = if guarded {
                    "if flag:\n    raise\nrecover(value)\n"
                } else {
                    "recover(value)\n"
                };
                let protected = format!(
                    "try:\n    async for item in stream(value):\n        await prepare(item)\n    if flag:\n        {terminal}\n{handler}finally:\n    try:\n        await finish(value)\n    except Error:\n{}",
                    indent(&indent(cleanup_handler)),
                );
                for body in [
                    protected.clone(),
                    format!("for item in values:\n{}", indent(&protected)),
                    format!("async with context():\n{}", indent(&protected)),
                ] {
                    let source = format!(
                        "async def f(value, flag, values):\n{}    complete(value)\n",
                        indent(&body),
                    );
                    assert_expression_cleanup_matches_cpython(
                        &source,
                        "awaited_normal_finally_locations.py",
                    );
                }
            }
        }
    }
}

#[test]
fn expression_cleanup_preserves_the_original_unlocated_manager_join() {
    for (definition, generator) in [
        ("def", false),
        ("def", true),
        ("async def", false),
        ("async def", true),
    ] {
        let asynchronous = definition == "async def";
        let cleanup = if asynchronous {
            "await cleanup(value)"
        } else {
            "cleanup(value)"
        };
        for protected in [
            "with nested:\n    return\n",
            "with nested:\n    raise Error(value)\n",
        ] {
            for prefix in ["", "value = choose(value)\n"] {
                for guard in ["flag", "flag or value"] {
                    let finalbody = format!("{prefix}if {guard}:\n    {cleanup}\n");
                    let inner = format!(
                        "try:\n{}finally:\n{}",
                        indent(protected),
                        indent(&finalbody)
                    );
                    let manager = format!("with manager:\n{}", indent(&inner));
                    for mut body in [
                        manager.clone(),
                        format!(
                            "try:\n{}except Error:\n    recover(value)\n",
                            indent(&manager)
                        ),
                    ] {
                        if generator {
                            body.insert_str(0, "yield value\n");
                        }
                        let source = format!(
                            "{definition} f(value, flag, manager, nested):\n{}",
                            indent(&body)
                        );
                        assert_expression_cleanup_matches_cpython(
                            &source,
                            "expression_finally_manager_join.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn expression_cleanup_retains_outer_async_manager_identities() {
    for generator in [false, true] {
        for prefix in ["", "value = choose(value)\n"] {
            let finalbody =
                format!("{prefix}if flag:\n    prepare(value)\n    await cleanup(value)\n");
            let inner = format!(
                "with manager:\n    try:\n        with nested:\n            raise Error(value)\n    finally:\n{}",
                indent(&indent(&finalbody))
            );
            for wrapped in [
                inner.clone(),
                format!("async with second:\n{}", indent(&inner)),
                format!("with second:\n{}", indent(&inner)),
            ] {
                let mut body = format!("async with first:\n{}", indent(&wrapped));
                if generator {
                    body.insert_str(0, "yield value\n");
                }
                let source = format!(
                    "async def f(value, flag, manager, nested, first, second):\n{}",
                    indent(&body)
                );
                assert_expression_cleanup_matches_cpython(
                    &source,
                    "retained_outer_async_managers.py",
                );
            }
        }
    }
}

#[test]
fn protected_finally_preserves_the_original_manager_join_owner() {
    for (definition, generator) in [
        ("def", false),
        ("def", true),
        ("async def", false),
        ("async def", true),
    ] {
        let asynchronous = definition == "async def";
        let awaited = if asynchronous { "await " } else { "" };
        let inner = format!(
            "with manager:\n    try:\n        with nested:\n            raise Error(value)\n    finally:\n        if flag:\n            {awaited}cleanup(value)\n"
        );
        let outer_cleanup = format!("{awaited}outer_cleanup(value)\n");
        let mut variants = vec![
            (inner.clone(), outer_cleanup.clone()),
            (format!("before(value)\n{inner}"), outer_cleanup.clone()),
            (format!("{inner}after(value)\n"), outer_cleanup.clone()),
            (inner.clone(), "outer_cleanup(value)\n".to_string()),
        ];
        if asynchronous {
            variants.extend([
                (
                    format!("async with first:\n{}", indent(&inner)),
                    outer_cleanup.clone(),
                ),
                (
                    format!(
                        "async with first:\n    async with second:\n{}",
                        indent(&indent(&inner))
                    ),
                    outer_cleanup,
                ),
            ]);
        }
        for (protected, finalbody) in variants {
            let mut body = format!(
                "try:\n{}finally:\n{}",
                indent(&protected),
                indent(&finalbody)
            );
            if generator {
                body.insert_str(0, "yield value\n");
            }
            let source = format!(
                "{definition} f(value, flag, manager, nested, first, second):\n{}",
                indent(&body)
            );
            assert_expression_cleanup_matches_cpython(&source, "protected_finally_manager_join.py");
        }
    }
}

#[test]
fn protected_finally_preserves_manager_joins_in_module_and_class_scopes() {
    for prefix in ["", "value = choose(value)\n"] {
        let finalbody = format!("{prefix}if flag:\n    cleanup(value)\n");
        let inner = format!(
            "with manager:\n    try:\n        with nested:\n            raise Error(value)\n    finally:\n{}",
            indent(&indent(&finalbody))
        );
        let body = format!(
            "try:\n{}finally:\n    outer_cleanup(value)\n",
            indent(&inner)
        );
        for source in [body.clone(), format!("class C:\n{}", indent(&body))] {
            assert_expression_cleanup_matches_cpython(&source, "protected_scope_manager_join.py");
        }
    }
}

#[test]
fn async_iteration_before_expression_cleanup_keeps_the_manager_join_owner() {
    let source = r"async def f(values, flag, manager):
    with manager:
        try:
            async for value in values:
                yield value
        finally:
            if flag:
                await cleanup(value)
";
    assert_expression_cleanup_matches_cpython(source, "async_iteration_finally_manager_join.py");
}

#[test]
fn completed_contexts_preserve_the_original_manager_join_owner() {
    for (definition, protected) in [
        ("def", "work(value)"),
        ("async def", "await work(value)"),
        ("def", "yield value"),
        ("async def", "yield value"),
    ] {
        for prefix in [
            "prepare(value)\n",
            "with other:\n    prepare(value)\n",
            "for item in values:\n    prepare(item)\n",
        ] {
            for cleanup in [
                "try:\n    finish(value)\nexcept Error:\n    pass\n",
                "with other:\n    finish(value)\n",
            ] {
                let finalbody = format!("{prefix}if flag:\n{}", indent(cleanup));
                let inner = format!(
                    "with manager:\n    try:\n        {protected}\n    finally:\n{}",
                    indent(&indent(&finalbody))
                );
                for body in [
                    inner.clone(),
                    format!("for value in values:\n{}", indent(&inner)),
                    format!("with outer:\n{}", indent(&inner)),
                ] {
                    let source = format!(
                        "{definition} f(value, flag, values, manager, other, outer):\n{}    finish_tail(value)\n",
                        indent(&body)
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "completed_finally_contexts.py",
                    );
                }
            }
        }
    }
}

#[test]
fn completed_managers_inside_recovery_keep_the_outer_join_owner() {
    let source = r"def f(value, flag, manager, other):
    with manager:
        try:
            work(value)
        finally:
            if flag:
                with other:
                    prepare(value)
                try:
                    finish(value)
                except Error:
                    with other:
                        recover(value)
                    done(value)
    finish_tail(value)
";
    common::assert_matches_cpython_marshal(source, "nested_completed_finally_managers.py");
}

#[test]
fn awaited_recovery_in_an_async_generator_keeps_the_manager_join_owner() {
    let source = r"async def f(value, flag, manager):
    with manager:
        try:
            yield value
        finally:
            if flag:
                prepare(value)
                before(value)
                try:
                    await finish(value)
                except Error:
                    recover(value)
                    await done(value)
    finish_tail(value)
";
    common::assert_matches_cpython_marshal(source, "async_generator_finally_recovery.py");
}

#[test]
fn yielding_headers_keep_their_existing_manager_ownership() {
    for keyword in ["yield", "yield from"] {
        let recovery = "try:\n    finish(value)\nexcept Error:\n    recover(value)\n";
        for finalbody in [
            format!("if ({keyword} value):\n{}", indent(recovery)),
            format!(
                "if ({keyword} value):\n    prepare(value)\nif flag:\n{}",
                indent(recovery)
            ),
            format!("if flag:\n    with ({keyword} manager):\n        finish(value)\n"),
        ] {
            let body = format!(
                "with manager:\n    try:\n        yield value\n    finally:\n{}",
                indent(&indent(&finalbody))
            );
            let source = format!(
                "def f(value, flag, manager):\n{}    tail(value)\n",
                indent(&body)
            );
            common::assert_matches_cpython_marshal(&source, "yielding_finally_headers.py");
        }
    }
}

#[test]
fn completed_guard_preserves_manager_owner_on_later_cleanup_exit() {
    for asynchronous in [false, true] {
        for cleanup in [
            "pass",
            "work()",
            "try:\n                    work()\n                except Exception:\n                    handle()",
        ] {
            let definition = if asynchronous { "async def" } else { "def" };
            let suspension = if asynchronous { "await " } else { "" };
            let source = format!(
                "{definition} f(first, second):\n    with manager():\n        try:\n            {suspension}work()\n        except Exception:\n            handle()\n        finally:\n            if first is not None:\n                {cleanup}\n            if second:\n                try:\n                    work()\n                    {suspension}close()\n                except Exception as error:\n                    handle(error)\n"
            );
            common::assert_matches_cpython_marshal(&source, "completed_finally_guard.py");
        }
    }
}

#[test]
fn awaited_guarded_recovery_preserves_manager_owner_on_later_cleanup_exit() {
    let source = r"async def f(first, second):
    with manager():
        try:
            await work()
        except Exception as error:
            handle(error)
        finally:
            if first is not None:
                try:
                    await close()
                except Exception:
                    handle()
            if second:
                try:
                    work()
                    await close()
                except Exception as error:
                    handle(error)
";
    common::assert_matches_cpython_marshal(source, "awaited_finally_guards.py");
}
