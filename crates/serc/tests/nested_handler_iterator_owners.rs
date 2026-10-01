#[allow(dead_code)]
mod common;

use std::fmt::Write as _;
use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_with_options};

fn assert_matches_cpython(version: PythonVersion, source: &str) {
    let (variable, fallback) = match version {
        PythonVersion::Py312 => ("SERC_TEST_PYTHON_312", "python3.12"),
        PythonVersion::Py313 => ("SERC_TEST_PYTHON_313", "python3.13"),
        PythonVersion::Py314 => ("SERC_TEST_PYTHON", "python3.14"),
        PythonVersion::Py315 => ("SERC_TEST_PYTHON_315", "python3.15"),
    };
    let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
    let target = version.target();
    let mut child = Command::new(python)
        .args(["-I", "-S", "-B", "-c", "import importlib.util, marshal, sys; assert sys.implementation.name == 'cpython'; assert sys.version.split()[0] == sys.argv[1]; assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]; code = compile(sys.stdin.read(), 'nested_handler_iterator.py', 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))"])
        .args([target.tag.trim_start_matches('v'), &target.magic_hex()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the pinned CPython oracle");
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
    let actual = compile_with_options(source, "nested_handler_iterator.py", &options)
        .unwrap()
        .marshal();
    assert!(actual == expected.stdout, "{version:?}:\n{source}");
}

#[test]
fn managed_handler_returns_retire_only_tries_inside_the_iterator() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (scope, prefix, work) in [
            ("def", "", "work()"),
            ("async def", "", "await work()"),
            ("def", "yield value\n", "work()"),
        ] {
            for name in ["", " as error"] {
                for early in ["", "except Early:\n    recover()\n"] {
                    for returned in ["False", "value"] {
                        for shape in ["direct", "older_try", "older_manager_try", "for_else"] {
                            let mut body = format!(
                                "try:\n    {work}\n{early}except Error{name}:\n    if flag:\n        return {returned}\n    continue\n"
                            );
                            body = format!(
                                "try:\n{}except Outer:\n    raise\n",
                                common::indent(&body)
                            );
                            body = format!("for item in items:\n{}", common::indent(&body));
                            if shape == "for_else" {
                                body.push_str("else:\n    finish()\n");
                            }
                            if shape == "older_try" {
                                body = format!(
                                    "try:\n{}except Older:\n    raise\n",
                                    common::indent(&body)
                                );
                            }
                            body = format!("with manager:\n{}", common::indent(&body));
                            if shape == "older_manager_try" {
                                body = format!(
                                    "try:\n{}except Older:\n    raise\n",
                                    common::indent(&body)
                                );
                            }
                            let source = format!(
                                "{scope} f(items, flag, value, manager):\n{}{}",
                                common::indent(prefix),
                                common::indent(&body)
                            );
                            assert_matches_cpython(version, &source);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn retained_try_chains_own_copied_finalizer_work() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (scope, prefix, cleanup) in [
            ("def", "", "cleanup()"),
            ("async def", "", "await cleanup()"),
            ("def", "yield value\n", "cleanup()"),
        ] {
            for count in [2, 3] {
                for returned in ["False", "value"] {
                    for pattern in ["plain", "iterator", "managed_handler"] {
                        let mut body = format!("if flag:\n    return {returned}\nwork()\n");
                        if pattern == "managed_handler" {
                            body = format!(
                                "with manager:\n    try:\n        work()\n    except ReturnError:\n{}",
                                common::indent(&common::indent(&body))
                            );
                        }
                        let finalbody = if pattern == "iterator" {
                            format!(
                                "try:\n    {cleanup}\nexcept CleanupError:\n    {cleanup}\n    raise\n"
                            )
                        } else {
                            format!("{cleanup}\n")
                        };
                        body = format!(
                            "try:\n{}finally:\n{}",
                            common::indent(&body),
                            common::indent(&finalbody)
                        );
                        if pattern == "iterator" {
                            body = format!("for item in items:\n{}", common::indent(&body));
                        }
                        for index in 0..count {
                            body = format!(
                                "try:\n{}except Middle{index}:\n    recover()\n",
                                common::indent(&body)
                            );
                        }
                        body = format!("try:\n{}finally:\n    reset()\n", common::indent(&body));
                        let source = format!(
                            "{scope} f(items, flag, value, manager):\n{}{}",
                            common::indent(prefix),
                            common::indent(&body)
                        );
                        assert_matches_cpython(version, &source);
                    }
                }
            }
        }
    }
}

#[test]
fn adjacent_handlers_keep_the_exposed_cleanup_owner() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (scope, work) in [
            ("def", "work()"),
            ("async def", "await work()"),
            ("def", "yield work()"),
        ] {
            for names in 0..8 {
                for returned in ["False", "value"] {
                    if returned == "value" && names != 3 {
                        continue;
                    }
                    for ambient in ["", "for item in items:", "while flag:"] {
                        let mut body = format!("return {returned}\n");
                        for index in (0..3).rev() {
                            let name = if names & (1 << index) == 0 {
                                String::new()
                            } else {
                                format!(" as error{index}")
                            };
                            body = format!(
                                "try:\n    {work}\nexcept Error{index}{name}:\n{}",
                                common::indent(&body),
                            );
                        }
                        if !ambient.is_empty() {
                            body = format!("{ambient}\n{}", common::indent(&body));
                        }
                        let source =
                            format!("{scope} f(items, flag, value):\n{}", common::indent(&body));
                        assert_matches_cpython(version, &source);
                    }
                }
            }
        }
    }
}

#[test]
fn live_handlers_own_linear_finalizers_before_their_cleanup() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (scope, prefix) in [
            ("def", ""),
            ("async def", "await initial()\n"),
            ("def", "yield value\n"),
        ] {
            for returned in ["return", "return value"] {
                for name in ["", " as error"] {
                    for handlers in [
                        "",
                        "except Other:\n    raise\nexcept Another as inner_error:\n    recover(inner_error)\n",
                    ] {
                        for cleanup in ["value.flag = False\n", "cleanup(value)\n"] {
                            let returning = format!("if flag:\n    {returned}\nwork(value)\n");
                            let inner = format!(
                                "try:\n{}{handlers}finally:\n{}",
                                common::indent(&returning),
                                common::indent(cleanup)
                            );
                            let body = format!(
                                "try:\n    work(value)\nexcept Error{name}:\n{}finally:\n    finish()\n",
                                common::indent(&inner)
                            );
                            for wrapper in ["plain", "managed_while", "while_managed"] {
                                let body = match wrapper {
                                    "managed_while" => format!(
                                        "with manager:\n{}",
                                        common::indent(&format!(
                                            "while flag:\n{}after()\n",
                                            common::indent(&body)
                                        ))
                                    ),
                                    "while_managed" => format!(
                                        "while flag:\n{}after()\n",
                                        common::indent(&format!(
                                            "with manager:\n{}",
                                            common::indent(&body)
                                        ))
                                    ),
                                    _ => body.clone(),
                                };
                                let source = format!(
                                    "{scope} f(value, flag, manager):\n{}",
                                    common::indent(&format!("{prefix}{body}"))
                                );
                                assert_matches_cpython(version, &source);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn contiguous_handler_swaps_keep_each_common_cleanup() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (scope, prefix, work) in [
            ("def", "", "work(value)"),
            ("async def", "", "work(value)"),
            ("async def", "", "await work(value)"),
            ("def", "yield value\n", "work(value)"),
        ] {
            for (count, naming) in [
                (2, "none"),
                (2, "inner"),
                (2, "outer"),
                (2, "all"),
                (3, "inner"),
                (3, "all"),
            ] {
                for returned in ["value", "result(value)", "None"] {
                    let mut body = format!("return {returned}\n");
                    for index in (0..count).rev() {
                        let named = naming == "all"
                            || (naming == "inner" && index + 1 == count)
                            || (naming == "outer" && index == 0);
                        let name = if named {
                            format!(" as error_{index}")
                        } else {
                            String::new()
                        };
                        body = format!(
                            "try:\n    {work}\nexcept Error{name}:\n{}",
                            common::indent(&body)
                        );
                    }
                    let source = format!(
                        "{scope} f(value):\n{}",
                        common::indent(&format!("{prefix}{body}after()\n"))
                    );
                    assert_matches_cpython(version, &source);
                }
            }
        }
    }
}

#[test]
fn handler_swap_owners_preserve_interleaved_unwind_scopes() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (scope, prefix) in [("def", ""), ("async def", ""), ("def", "yield value\n")] {
            for returned in ["result(value)", "None"] {
                for (boundary, position, naming) in [
                    ("try", "inner", "all"),
                    ("finally", "outer", "all"),
                    ("while", "middle", "all"),
                    ("for", "middle", "inner"),
                    ("manager", "outer", "inner"),
                    ("manager", "middle", "inner"),
                    ("manager", "inner", "inner"),
                ] {
                    let wrap = |body: &str| match boundary {
                        "try" => format!(
                            "try:\n{}except Other:\n    recover()\n",
                            common::indent(body)
                        ),
                        "finally" => {
                            format!("try:\n{}finally:\n    cleanup()\n", common::indent(body))
                        }
                        "while" => format!("while flag:\n{}", common::indent(body)),
                        "for" => format!("for item in values:\n{}", common::indent(body)),
                        "manager" => format!("with manager:\n{}", common::indent(body)),
                        _ => unreachable!(),
                    };
                    let mut body = format!("return {returned}\n");
                    if position == "inner" {
                        body = wrap(&body);
                    }
                    for index in (0..3).rev() {
                        let name = if naming == "all" || index == 2 {
                            format!(" as error_{index}")
                        } else {
                            String::new()
                        };
                        body = format!(
                            "try:\n    work(value)\nexcept Error{name}:\n{}",
                            common::indent(&body)
                        );
                        if index == 1 && position == "middle" {
                            body = wrap(&body);
                        }
                    }
                    if position == "outer" {
                        body = wrap(&body);
                    }
                    let source = format!(
                        "{scope} f(value, manager, flag, values):\n{}",
                        common::indent(&format!("{prefix}{body}after()\n"))
                    );
                    assert_matches_cpython(version, &source);
                }
            }
        }
    }
}

#[test]
fn nested_finally_manager_returns_preserve_original_null_cells() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for scope in ["def", "async def"] {
            for returned in ["return", "return False"] {
                for count in [0, 1, 2, 8] {
                    let body = format!("{}{returned}\n", "log(value)\n".repeat(count));
                    let source = format!(
                        "{scope} f(manager, values, flag):\n    try:\n        with manager:\n            for value in values:\n                if flag:\n{}    except Error:\n        recover()\n    finally:\n        try:\n            if flag:\n                cleanup()\n        finally:\n            finish()\n",
                        common::indent(&common::indent(&common::indent(&common::indent(
                            &common::indent(&body)
                        ))))
                    );
                    assert_matches_cpython(version, &source);
                }
            }
        }
    }
}

#[test]
fn nested_finally_manager_returns_preserve_pruned_constant_identity() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for scope in ["def", "async def"] {
            for prefix in [
                "value = 'first' + 'second'\n",
                "value = (1, 2, 3)\n",
                "if False:\n    log('unused')\n",
            ] {
                for condition in ["flag", "flag is not None"] {
                    let source = format!(
                        "{scope} f(manager, values, flag, value):\n{}    try:\n        with manager:\n            for item in values:\n                if flag:\n                    log('retained', key=True)\n                    return False\n    except Error:\n        recover()\n    finally:\n        try:\n            if {condition}:\n                cleanup()\n        finally:\n            finish()\n",
                        common::indent(prefix)
                    );
                    assert_matches_cpython(version, &source);
                }
            }
        }
    }
}

#[test]
fn unnamed_handler_returns_preserve_mixed_original_cells() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (scope, prefix, returned) in [
            ("def", "", "return"),
            ("def", "", "return value"),
            ("async def", "", "return"),
            ("async def", "", "return value"),
            ("def", "yield value\n", "return"),
            ("def", "yield value\n", "return value"),
            ("async def", "yield value\n", "return"),
        ] {
            for count in [2, 4] {
                for handler in ["", " as error"] {
                    for outer in [false, true] {
                        for call in ["log(value)\n", "log(value, key=True)\n"] {
                            let body = format!(
                                "try:\n    work()\nexcept Error{handler}:\n{}",
                                common::indent(&format!("{}{returned}\n", call.repeat(count)))
                            );
                            let body = if outer {
                                format!(
                                    "try:\n{}except OtherError:\n    pass\n",
                                    common::indent(&body)
                                )
                            } else {
                                body
                            };
                            let body = format!(
                                "{prefix}try:\n{}finally:\n    if value is not None:\n        cleanup()\n",
                                common::indent(&body)
                            );
                            let source = format!("{scope} f(value):\n{}", common::indent(&body));
                            assert_matches_cpython(version, &source);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn nested_finally_returns_preserve_adjacent_pop_and_setup_cells() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (scope, prefix) in [
            ("def", ""),
            ("async def", ""),
            ("async def", "value = await initial(value)\n"),
            ("def", "yield value\n"),
        ] {
            // These counts place the appended marker on an inner-owned record,
            // the SETUP, the preceding POP, and two outer-owned records.
            for count in 2..=6 {
                for returned in ["value", "result(value)"] {
                    let returning = format!(
                        "{prefix}{}return {returned}\n",
                        "value = work(value)\n".repeat(count)
                    );
                    let body = format!(
                        "try:\n{}finally:\n    try:\n        if flag:\n            pass\n    finally:\n        cleanup()\n",
                        common::indent(&returning)
                    );
                    for wrapper in ["plain", "except", "with", "loop", "async with"] {
                        if wrapper == "async with" && scope != "async def" {
                            continue;
                        }
                        let wrapped = match wrapper {
                            "except" => format!(
                                "try:\n{}except Error:\n    recover()\n",
                                common::indent(&body)
                            ),
                            "with" | "async with" => {
                                format!("{wrapper} manager():\n{}", common::indent(&body))
                            }
                            "loop" => format!("for item in items:\n{}", common::indent(&body)),
                            _ => body.clone(),
                        };
                        let source =
                            format!("{scope} f(value, flag):\n{}", common::indent(&wrapped));
                        assert_matches_cpython(version, &source);
                    }
                }
            }
        }
    }
}

#[test]
fn nested_finally_returns_preserve_consecutive_try_pop_cells() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (count, returned) in [
            (1, "result(value)"),
            (2, "value"),
            (2, "result(value)"),
            (3, "value"),
            (3, "result(value)"),
            (4, "value"),
        ] {
            let source = format!(
                "async def f(value, flag):\n    try:\n        value = await initial(value)\n{}        return {returned}\n    except Error as error:\n        recover()\n    finally:\n        try:\n            if flag:\n                pass\n        finally:\n            cleanup()\n",
                "        value = work(value)\n".repeat(count)
            );
            assert_matches_cpython(version, &source);
        }
    }
}

#[test]
fn manager_inside_handler_preserves_mixed_cleanup_cells() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for scope in ["def", "async def"] {
            for returned in [
                "value",
                "(value,)",
                "(value, value)",
                "result(value)",
                "(value, {}, first, second, None, third)",
            ] {
                for outer in [false, true] {
                    for guard in ["is None", "is not None"] {
                        for finish in ["pass", "finish()"] {
                            let condition = if scope == "async def" {
                                "await condition()"
                            } else {
                                "condition()"
                            };
                            let protected = format!(
                                "if value is not None:\n    try:\n        pass\n    except Error:\n        with manager:\n            if {condition}:\n                return {returned}\n"
                            );
                            let protected = if outer {
                                format!(
                                    "try:\n{}except OtherError:\n    pass\n",
                                    common::indent(&protected)
                                )
                            } else {
                                protected
                            };
                            let source = format!(
                                "{scope} f(manager, value):\n    try:\n{}    finally:\n        if value {guard}:\n            {finish}\n",
                                common::indent(&common::indent(&protected))
                            );
                            assert_matches_cpython(version, &source);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn manager_else_return_preserves_the_erased_inner_try_cell() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (scope, inner) in [
            ("def", "with"),
            ("async def", "with"),
            ("async def", "async with"),
        ] {
            for assignment in [
                "",
                "value.attr = None\n",
                "copy = value\n",
                "value.attr = value\n",
            ] {
                for guard in ["is None", "is not None"] {
                    for cleanup in ["pass", "finish()"] {
                        let returned = common::indent(&format!("{assignment}return value\n"));
                        let source = format!(
                            "{scope} f(manager, timeout, value):\n    try:\n        try:\n            with manager:\n                try:\n                    {inner} timeout:\n                        pass\n                except Error:\n                    pass\n                else:\n{}        except OtherError:\n            pass\n    finally:\n        if value {guard}:\n            {cleanup}\n",
                            common::indent(&common::indent(&common::indent(&common::indent(
                                &returned
                            ))))
                        );
                        assert_matches_cpython(version, &source);
                    }
                }
            }
        }
    }
}

#[test]
fn inner_try_retires_before_nested_handler_iterators() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for scope in ["def", "async def"] {
            for handler_depth in [1, 2, 3] {
                for loop_depth in [1, 2] {
                    for named_outer in [false, true] {
                        for returned in ["return", "return result()"] {
                            let mut body =
                                format!("try:\n    {returned}\nexcept LastError:\n    pass\n");
                            for index in 0..loop_depth {
                                body =
                                    format!("for item{index} in items:\n{}", common::indent(&body));
                            }
                            for index in 0..handler_depth {
                                let name = if named_outer && index == handler_depth - 1 {
                                    " as error"
                                } else {
                                    ""
                                };
                                body = format!(
                                    "try:\n    work()\nexcept Error{name}:\n{}",
                                    common::indent(&body)
                                );
                            }
                            let source = format!("{scope} f(items):\n{}", common::indent(&body));
                            assert_matches_cpython(version, &source);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn inner_try_retires_before_manager_iterators_and_finally() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for manager in ["with", "async with"] {
            for returned in ["value", "result()", "await result()"] {
                for loop_depth in [1, 2] {
                    for cleanup in ["pass", "cleanup()"] {
                        let mut body = format!(
                            "try:\n    return {returned}\nexcept Error as error:\n    pass\n"
                        );
                        for index in 0..loop_depth {
                            body = format!("for item{index} in items:\n{}", common::indent(&body));
                        }
                        body = format!("{manager} manager:\n{}", common::indent(&body));
                        body = format!("try:\n{}finally:\n    {cleanup}\n", common::indent(&body));
                        let source = format!(
                            "async def f(manager, items, value):\n{}",
                            common::indent(&body)
                        );
                        assert_matches_cpython(version, &source);
                    }
                }
            }
        }
    }
}

#[test]
fn nested_handler_swaps_precede_the_outer_finally_unwind() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for scope in ["def", "async def"] {
            for handler_depth in [2, 3] {
                for returned in ["value", "result()"] {
                    for cleanup in ["pass", "cleanup()"] {
                        let mut body = format!("return {returned}\n");
                        for _ in 0..handler_depth {
                            body = format!(
                                "try:\n    work()\nexcept Error:\n{}",
                                common::indent(&body)
                            );
                        }
                        writeln!(body, "finally:\n    {cleanup}").unwrap();
                        let source = format!("{scope} f(value):\n{}", common::indent(&body));
                        assert_matches_cpython(version, &source);
                    }
                }
            }
        }
    }
}

#[test]
fn copied_handler_finally_keeps_the_intervening_try_owner() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for scope in ["def", "async def"] {
            for name in ["", " as error"] {
                for returned in ["None", "value", "result()"] {
                    for outer_cleanup in ["pass", "finish()"] {
                        let cleanup = if scope == "async def" {
                            "await cleanup()"
                        } else {
                            "cleanup()"
                        };
                        let body = format!(
                            "try:\n    try:\n        work()\n    except Error{name}:\n        record()\n        return {returned}\n    finally:\n        {cleanup}\nexcept OtherError:\n    pass\nfinally:\n    {outer_cleanup}\n"
                        );
                        let source = format!("{scope} f(value):\n{}", common::indent(&body));
                        assert_matches_cpython(version, &source);
                    }
                }
            }
        }
    }
}

#[test]
fn handler_return_keeps_the_enclosing_try_with_older_finalizers() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for scope in ["def", "async def"] {
            for managers in ["manager", "manager, other_manager"] {
                for older_finalizers in [0, 1, 2] {
                    for returned in ["value", "result(value, error)"] {
                        for cleanup in ["pass", "cleanup()"] {
                            for guarded in [false, true] {
                                let action = if guarded {
                                    format!("if flag:\n    return {returned}\n")
                                } else {
                                    format!("return {returned}\n")
                                };
                                let body = format!(
                                    "try:\n    work()\nexcept Error as error:\n{}",
                                    common::indent(&action)
                                );
                                let body = format!(
                                    "try:\n{}except OtherError as other:\n    recover()\nfinally:\n    {cleanup}\n",
                                    common::indent(&body)
                                );
                                let mut body =
                                    format!("with {managers}:\n{}", common::indent(&body));
                                for _ in 0..older_finalizers {
                                    body = format!(
                                        "try:\n{}finally:\n    {cleanup}\n",
                                        common::indent(&body)
                                    );
                                }
                                let source = format!(
                                    "{scope} f(manager, other_manager, value, flag):\n{}",
                                    common::indent(&body)
                                );
                                assert_matches_cpython(version, &source);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn manager_handler_return_keeps_its_ordinary_try_owner() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for scope in ["def", "async def"] {
            for manager in [
                "with manager",
                "with manager, other_manager",
                "async with manager",
            ] {
                if scope == "def" && manager.starts_with("async") {
                    continue;
                }
                for name in ["", " as error"] {
                    for returned in ["return", "return result(value)"] {
                        for cleanup in ["pass", "cleanup()"] {
                            for inner_manager in [false, true] {
                                let mut action = format!("if flag:\n    {returned}\n");
                                if inner_manager {
                                    action = format!("{manager}:\n{}", common::indent(&action));
                                }
                                let mut body = format!(
                                    "try:\n    work()\nexcept Error{name}:\n{}",
                                    common::indent(&action)
                                );
                                if !inner_manager {
                                    body = format!("while flag:\n{}", common::indent(&body));
                                }
                                body = format!(
                                    "try:\n{}except OtherError:\n    recover()\nfinally:\n    {cleanup}\n",
                                    common::indent(&body)
                                );
                                if !inner_manager {
                                    body = format!("{manager}:\n{}", common::indent(&body));
                                }
                                let source = format!(
                                    "{scope} f(manager, other_manager, value, flag):\n{}",
                                    common::indent(&body)
                                );
                                assert_matches_cpython(version, &source);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn nested_handlers_in_while_keep_the_enclosing_try_owner() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for scope in ["def", "async def"] {
            for outer_name in ["", " as outer_error"] {
                for inner_name in ["", " as inner_error"] {
                    for returned in ["return", "return result(value)"] {
                        for loop_depth in [1, 2] {
                            let mut body = format!(
                                "try:\n    work()\nexcept Error{outer_name}:\n    try:\n        work()\n    except OtherError{inner_name}:\n        {returned}\n"
                            );
                            for _ in 0..loop_depth {
                                body = format!("while flag:\n{}", common::indent(&body));
                            }
                            let source = format!(
                                "{scope} f(value, flag):\n    try:\n{}    except LastError:\n        recover()\n",
                                common::indent(&common::indent(&body))
                            );
                            assert_matches_cpython(version, &source);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn raising_handlers_in_copied_finalizers_keep_the_intervening_try() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for scope in ["def", "async def"] {
            for name in ["", " as error"] {
                for returned in ["value", "result(value)"] {
                    for cleanup in ["pass", "cleanup()"] {
                        let source = format!(
                            "{scope} f(value):\n    try:\n        try:\n            return {returned}\n        except OtherError{name}:\n            recover()\n        finally:\n            try:\n                work()\n            except Error{name}:\n                if value is None:\n                    raise\n    except LastError:\n        recover()\n    finally:\n        {cleanup}\n"
                        );
                        assert_matches_cpython(version, &source);
                    }
                }
            }
        }
    }
}

#[test]
fn boolean_comparison_slots_are_counted_once() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for operator in ["<", "<=", "==", "!=", ">=", ">"] {
            for comparison in [
                format!("{{1, 2}} {operator} set(value)"),
                format!("not ({{1, 2}} {operator} set(value))"),
            ] {
                for expression in [
                    format!("flag or {comparison} or fallback()"),
                    format!("{comparison} or fallback()"),
                ] {
                    let source = format!(
                        "def f(items, flag):\n    return any({expression} for value in items)\n"
                    );
                    assert_matches_cpython(version, &source);
                }
            }
        }
    }
}

#[test]
fn suspended_handler_finalizers_retain_outer_collectors() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for generator in [false, true] {
            for name in ["", " as error"] {
                for cleanup in ["cleanup()", "await cleanup()"] {
                    let source = format!(
                        "async def f(value):\n    try:\n        try:\n            work()\n        except Error{name}:\n            record()\n            return\n        finally:\n            {cleanup}\n    except OtherError:\n        pass\n    finally:\n        pass\n{}",
                        if generator { "    yield value\n" } else { "" },
                    );
                    assert_matches_cpython(version, &source);
                }
            }
        }
        for loop_header in ["for item in items:", "while flag:"] {
            for cleanup in ["cleanup()", "await cleanup()"] {
                for outer_cleanup in ["pass", "finish()"] {
                    let source = format!(
                        "async def f(items, flag):\n    try:\n        {loop_header}\n            try:\n                work()\n            except Error as error:\n                record()\n                return\n            finally:\n                {cleanup}\n    except OtherError:\n        pass\n    finally:\n        {outer_cleanup}\n"
                    );
                    assert_matches_cpython(version, &source);
                }
            }
        }
    }
}

#[test]
fn annotated_keyword_defaults_keep_the_original_conditional_array() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for scope in ["def", "async def"] {
            for returned in ["None", "flag", "(flag, other)"] {
                for lambda_call in ["", "    service.call(lambda: Kind.VALUE)\n"] {
                    let source = format!(
                        "{scope} f(flag, other):\n    service.call([(*item, item[0].format('x')) for item in items])\n    {scope} nested(*, kind: Kind = Kind.VALUE) -> object:\n        return {returned}\n{lambda_call}    try:\n        if flag:\n            pass\n        else:\n            pass\n    finally:\n        pass\n"
                    );
                    assert_matches_cpython(version, &source);
                }
            }
        }
    }
}

#[test]
fn raising_finalizers_keep_the_intervening_try_owner() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for finalizer in [
            "if flag:\n    raise Error",
            "try:\n    finish()\nexcept Error:\n    raise",
        ] {
            for cleanup in ["cleanup()", "await cleanup()"] {
                for outer_cleanup in ["pass", "finish()"] {
                    let finalizer = common::indent(&common::indent(&common::indent(
                        &common::indent(&format!("{finalizer}\n{cleanup}\n")),
                    )));
                    let source = format!(
                        "async def f(items, flag):\n    try:\n        for item in items:\n            try:\n                work()\n            except Error as error:\n                return\n            finally:\n{finalizer}    except OtherError:\n        pass\n    finally:\n        {outer_cleanup}\n"
                    );
                    assert_matches_cpython(version, &source);
                }
            }
        }
    }
}

#[test]
fn async_comprehension_filters_keep_the_removed_try_pop() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (open, close, element) in [
            ("[", "]", "value"),
            ("{", "}", "value"),
            ("{", "}", "value: value.other"),
            ("(", ")", "value"),
        ] {
            for target in ["value", "(value, extra)"] {
                for condition in ["flag", "flag or value.other is None"] {
                    for multiline in [false, true] {
                        let expression = if multiline {
                            format!(
                                "{open}\n        {element}\n        async for {target} in items\n        if {condition}\n    {close}"
                            )
                        } else {
                            format!(
                                "{open}{element} async for {target} in items if {condition}{close}"
                            )
                        };
                        let source =
                            format!("async def f(items, flag):\n    return {expression}\n");
                        assert_matches_cpython(version, &source);
                    }
                }
            }
        }
    }
}
