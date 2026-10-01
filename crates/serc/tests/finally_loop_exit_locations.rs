use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

fn assert_matches_cpython(source: &str, version: PythonVersion) {
    let target = version.target();
    let (variable, fallback) = match version {
        PythonVersion::Py312 => ("SERC_TEST_PYTHON_312", "python3.12"),
        PythonVersion::Py313 => ("SERC_TEST_PYTHON_313", "python3.13"),
        PythonVersion::Py314 => ("SERC_TEST_PYTHON", "python3.14"),
        PythonVersion::Py315 => ("SERC_TEST_PYTHON_315", "python3.15"),
    };
    let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
    let oracle = r"
import importlib.util, marshal, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
code = compile(sys.stdin.buffer.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
    let filename = "finally_loop_exit_locations.py";
    let mut child = Command::new(python)
        .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle])
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
        "CPython oracle failed: {}",
        String::from_utf8_lossy(&expected.stderr)
    );
    let options = CompileOptions {
        python_version: version,
        ..CompileOptions::default()
    };
    for actual in [
        compile_with_options(source, filename, &options).unwrap(),
        compile_bytes_with_options(source.as_bytes(), filename, &options).unwrap(),
    ] {
        assert_eq!(actual.marshal(), expected.stdout, "{version:?}: {source}");
    }
}

#[test]
fn retained_finally_pops_preserve_owner_epochs_across_cold_region_ends() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for definition in ["def", "async def"] {
            for protected in [
                "consume(value)",
                "value = obj",
                "assert flag",
                "assert obj.ready()",
                "assert flag and other",
                "if not flag:\n    raise Error",
                "if flag:\n    raise Error",
            ] {
                for work in [
                    "consume(value)",
                    "first.finish()\nsecond.finish()",
                    "first.finish(limit=30)\nsecond.finish(limit=30)",
                ] {
                    for prefix in ["", "obj.release()\n"] {
                        for guard in ["flag", "not second.ready()"] {
                            let cleanup =
                                format!("if {guard}:\n    with receiver.manager:\n        pass");
                            let finalbody = format!(
                                "{prefix}try:\n    {}\nfinally:\n    {}",
                                work.replace('\n', "\n    "),
                                cleanup.replace('\n', "\n    ")
                            );
                            let body = format!(
                                "try:\n    {}\nfinally:\n    {}",
                                protected.replace('\n', "\n    "),
                                finalbody.replace('\n', "\n    ")
                            );
                            let source = format!(
                                "{definition} f(obj, receiver, value, flag, other, first, second):\n    with manager():\n        {}\n    finish(value)\n",
                                body.replace('\n', "\n        ")
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn retained_finally_pops_preserve_owner_epochs_across_split_cold_groups() {
    let prefixes = [
        "consume(value)\n",
        "try:\n    consume(value)\nexcept Error:\n    recover(value)\n",
        "try:\n    consume(value)\nfinally:\n    release(value)\n",
        "try:\n    consume(value)\nexcept Error:\n    raise\n",
        "with inner():\n    consume(value)\n",
        "if other:\n    consume(value)\n",
        "for item in values:\n    consume(item)\n",
        "try:\n    consume(value)\nexcept Error as error:\n    recover(error)\n",
    ];
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for definition in ["def", "async def"] {
            for (index, prefix) in prefixes.iter().enumerate() {
                let guards = if matches!(index, 1 | 3 | 7) {
                    &["flag and other"][..]
                } else {
                    &["flag", "obj.ready()", "flag and other"][..]
                };
                for depth in [1, 2] {
                    for guard in guards {
                        let protected = format!("{prefix}assert {guard}");
                        let finalbody = "obj.release()\ntry:\n    first.finish(limit=30)\n    second.finish(limit=30)\nfinally:\n    if not second.ready():\n        with receiver.manager:\n            receiver.notify()";
                        let mut body = format!(
                            "try:\n    {}\nfinally:\n    {}",
                            protected.replace('\n', "\n    "),
                            finalbody.replace('\n', "\n    ")
                        );
                        for level in 0..depth {
                            body = format!(
                                "with manager{level}():\n    {}",
                                body.replace('\n', "\n    ")
                            );
                        }
                        let source = format!(
                            "{definition} f(obj, receiver, value, flag, other, first, second, values):\n    {}\n    finish(value)\n",
                            body.replace('\n', "\n    ")
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn nested_finally_manager_pops_keep_shared_locations() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, prefix, manager) in [
            ("def", "", ""),
            ("def", "", "with"),
            ("def", "yield value\n", "with"),
            ("async def", "", "with"),
            ("async def", "", "async with"),
            ("async def", "yield value\n", "with"),
        ] {
            for depth in 0..4 {
                for alternate in ["", "\nelse:\n    pass", "\nelse:\n    consume(value)"] {
                    for trailing in ["", "\nconsume(value)"] {
                        let mut body =
                            format!("if flag:\n    with obj.manager:\n        pass{alternate}");
                        for _ in 0..depth {
                            body = format!(
                                "try:\n    consume(value)\nfinally:\n    {}",
                                body.replace('\n', "\n    ")
                            );
                        }
                        body.push_str(trailing);
                        let body = if manager.is_empty() {
                            format!("{prefix}{body}\nfinish(value)")
                        } else {
                            format!(
                                "{prefix}{manager} outer():\n    {}\nfinish(value)",
                                body.replace('\n', "\n    ")
                            )
                        };
                        let source = format!(
                            "{definition} f(obj, value, flag):\n    {}\n",
                            body.replace('\n', "\n    ")
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn located_finally_loop_exits_survive_earlier_cleanup_barriers() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, manager) in [
            ("def", ""),
            ("def", "with"),
            ("async def", ""),
            ("async def", "with"),
            ("async def", "async with"),
        ] {
            for recovery in [
                "except:\n    pass",
                "except ValueError:\n    pass",
                "except ValueError as error:\n    pass",
                "except ValueError as error:\n    value = error",
                "except ValueError as error:\n    if value is None:\n        value = error",
                "except ValueError as error:\n    raise RuntimeError from error",
            ] {
                for loop_body in [
                    "consume(value)",
                    "try:\n    consume(value)\nexcept ValueError:\n    pass",
                ] {
                    for outer_handler in ["", "except ValueError:\n    pass\n"] {
                        let mut body = format!(
                            "try:\n    consume(value)\n{outer_handler}finally:\n    for item in items:\n        try:\n            consume(item)\n        {}\n    while pending:\n        {}\nconsume(value)",
                            recovery.replace('\n', "\n        "),
                            loop_body.replace('\n', "\n        "),
                        );
                        if !manager.is_empty() {
                            body = format!(
                                "{manager} manager():\n    {}",
                                body.replace('\n', "\n    ")
                            );
                        }
                        let source = format!(
                            "{definition} f(value, items, pending):\n    {}\n",
                            body.replace('\n', "\n    "),
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn located_finally_loop_exits_keep_condition_and_continuation_boundaries() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, manager) in [
            ("def", ""),
            ("def", "with"),
            ("async def", ""),
            ("async def", "with"),
            ("async def", "async with"),
        ] {
            for prefix in [
                "for item in items:\n    consume(item)",
                "for item in items:\n    try:\n        consume(item)\n    except ValueError:\n        pass",
            ] {
                for condition in [
                    "pending",
                    "not pending",
                    "pending and flag",
                    "pending or flag",
                ] {
                    for tail in [
                        "consume(value)",
                        "return value",
                        "if flag:\n    return value\nconsume(pending)",
                    ] {
                        let mut body = format!(
                            "try:\n    consume(value)\nfinally:\n    {}\n    while {condition}:\n        consume(value)\n{tail}",
                            prefix.replace('\n', "\n    "),
                        );
                        if !manager.is_empty() {
                            body = format!(
                                "{manager} manager():\n    {}",
                                body.replace('\n', "\n    ")
                            );
                        }
                        let source = format!(
                            "{definition} f(value, items, pending, flag):\n    {}\n",
                            body.replace('\n', "\n    "),
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
                let mut body = format!(
                    "try:\n    consume(value)\nfinally:\n    {}\n    while pending and flag:\n        consume(value)",
                    prefix.replace('\n', "\n    "),
                );
                if !manager.is_empty() {
                    body = format!("{manager} manager():\n    {}", body.replace('\n', "\n    "));
                }
                let source = format!(
                    "{definition} f(value, items, pending, flag):\n    {}\n",
                    body.replace('\n', "\n    "),
                );
                assert_matches_cpython(&source, version);
            }
        }
    }
}

#[test]
fn linear_and_closed_guard_prefixes_keep_the_original_manager_join() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, work) in [
            ("def", "work(value)"),
            ("def", "yield value"),
            ("async def", "await work(value)"),
        ] {
            for leading in ["before(value)", "pass\nbefore(value)\npass"] {
                for earlier in [
                    "prepare(value)",
                    "prepare(value)\ntry:\n    first(value)\nexcept FirstError:\n    restore(value)\n    finish(value)",
                ] {
                    for condition in ["value is not None", "ready(value) is None"] {
                        for outer_manager in [false, true] {
                            let finalizer = format!(
                                "{leading}\nif flag:\n    {}\nif {condition}:\n    try:\n        cleanup(value)\n    except CleanupError:\n        pass",
                                earlier.replace('\n', "\n    "),
                            );
                            let mut body = format!(
                                "with manager:\n    try:\n        {work}\n    finally:\n        {}",
                                finalizer.replace('\n', "\n        "),
                            );
                            if outer_manager {
                                body = format!("with outer:\n    {}", body.replace('\n', "\n    "));
                            }
                            let source = format!(
                                "{definition} f(value, flag, manager, outer):\n    {}\n",
                                body.replace('\n', "\n    "),
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn completed_recovery_before_guarded_cleanup_keeps_the_manager_owner() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, work) in [
            ("def", "work(value)"),
            ("def", "yield value"),
            ("async def", "await work(value)"),
        ] {
            for condition in ["value is not None", "ready(value) is None"] {
                for prefix in ["prepare(value)", "value = prepare(value)"] {
                    for recovery in ["pass", "recover(value)\nfinish(value)"] {
                        for outer_finally in [false, true] {
                            let body = format!(
                                "try:\n    {work}\nfinally:\n    try:\n        {prefix}\n    except PrefixError:\n        pass\n    if {condition}:\n        try:\n            cleanup(value)\n        except CleanupError:\n            {}",
                                recovery.replace('\n', "\n            "),
                            );
                            let mut body =
                                format!("with manager:\n    {}", body.replace('\n', "\n    "));
                            if outer_finally {
                                body = format!(
                                    "with outer:\n    try:\n        {}\n    finally:\n        finish(value)",
                                    body.replace('\n', "\n        "),
                                );
                            }
                            let source = format!(
                                "{definition} f(value, manager, outer):\n    {}\n",
                                body.replace('\n', "\n    "),
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn completed_recovery_manager_joins_preserve_scope_and_ambient_owners() {
    let body = "with manager:\n    try:\n        work(value)\n    finally:\n        try:\n            prepare(value)\n        except PrefixError:\n            pass\n        if value is not None:\n            try:\n                cleanup(value)\n            except CleanupError:\n                recover(value)\n";
    let named_handler = body.replace(
        "    finally:",
        "    except Error as caught:\n        recover(value)\n    finally:",
    );
    let suspended = body.replace("work(value)", "await work(value)");
    for source in [
        body.to_owned(),
        format!("class C:\n    {}", body.replace('\n', "\n    ")),
        format!(
            "def f(value, manager):\n    {}",
            named_handler.replace('\n', "\n    "),
        ),
        format!(
            "async def f(value, manager, outer):\n    async with outer:\n        {}",
            suspended.replace('\n', "\n        "),
        ),
    ] {
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            assert_matches_cpython(&source, version);
        }
    }
}

#[test]
fn nested_capture_cleanup_preserves_the_outer_line_tail() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for inner_depth in 1..=4 {
            for leaf in ["value", "str(value)"] {
                let mut inner = leaf.to_owned();
                for index in 0..inner_depth {
                    inner = format!("{{'inner{index}': {inner}}}");
                }
                for split_outer_line in [false, true] {
                    let pattern = if split_outer_line {
                        format!("{{\n    'outer': {inner},\n}}")
                    } else {
                        format!("{{'outer': {inner}}}")
                    };
                    let matched = format!(
                        "match obj:\n    case {}:\n        use(value)\n    case _:\n        pass\nreturn value\n",
                        pattern.replace('\n', "\n    "),
                    );
                    for protection in ["plain", "except", "finally"] {
                        let body = match protection {
                            "except" => format!(
                                "try:\n    {}\nexcept Error:\n    recover(obj)\n",
                                matched.replace('\n', "\n    ").trim_end(),
                            ),
                            "finally" => format!(
                                "try:\n    {}\nfinally:\n    finish(obj)\n",
                                matched.replace('\n', "\n    ").trim_end(),
                            ),
                            _ => matched.clone(),
                        };
                        for definition in ["def", "async def"] {
                            let prefix = if definition == "async def" {
                                "    await start()\n"
                            } else {
                                ""
                            };
                            let source = format!(
                                "{definition} f(obj, value):\n{prefix}    {}",
                                body.replace('\n', "\n    "),
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn literal_and_sequence_captures_keep_split_cleanup_pop_locations() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for leaf in ["value", "str(value)"] {
            for split_outer_line in [false, true] {
                let inner = format!("{{'c': 'kind', 'd': [{{'e': {leaf}}}]}}");
                let pattern = if split_outer_line {
                    format!("{{\n    'a': 'ok',\n    'b': {inner},\n}}")
                } else {
                    format!("{{'a': 'ok', 'b': {inner}}}")
                };
                let matched = format!(
                    "match obj:\n    case {}:\n        use(value)\n    case _:\n        pass\nreturn value\n",
                    pattern.replace('\n', "\n    "),
                );
                for context in ["plain", "finally", "async"] {
                    let body = if context == "finally" {
                        format!(
                            "try:\n    {}\nfinally:\n    finish(obj)\n",
                            matched.replace('\n', "\n    ").trim_end(),
                        )
                    } else {
                        matched.clone()
                    };
                    let (definition, prefix) = if context == "async" {
                        ("async def", "    await start()\n")
                    } else {
                        ("def", "")
                    };
                    let source = format!(
                        "{definition} f(obj, value, other, flag):\n{prefix}    {}",
                        body.replace('\n', "\n    "),
                    );
                    assert_matches_cpython(&source, version);
                }
            }
        }
    }
}

#[test]
fn terminating_inner_handlers_keep_the_outer_finally_pop_line() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for definition in ["def", "async def"] {
            for protected in ["work(obj)", "value = obj"] {
                for recovery in ["return obj", "return None", "raise"] {
                    for tail in ["", "    return finish(obj)\n"] {
                        for finalbody in [
                            "try:\n    publish(obj)\nexcept Cleanup as caught:\n    raise Changed from caught",
                            "try:\n    publish(obj)\nfinally:\n    cleanup(obj)",
                        ] {
                            let source = format!(
                                "{definition} f(obj):\n    try:\n        try:\n            {protected}\n        except Inner:\n            {recovery}\n    except Error:\n        raise\n    finally:\n        {}\n{tail}",
                                finalbody.replace('\n', "\n        "),
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn finalizer_pop_donors_survive_completed_prefixes_and_suspended_recovery() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for before in [
            "",
            "with manager(obj):\n    prepare(obj)\n",
            "try:\n    prepare(obj)\nexcept Before:\n    recover(obj)\n",
        ] {
            for after in ["", "\nafter(obj)", "\npass"] {
                for handler in [
                    "await recover(obj)\nraise",
                    "if mode:\n    await recover(obj)\nraise",
                    "await recover(obj)\nreturn obj",
                ] {
                    let body = format!(
                        "{before}try:\n    work(obj)\nexcept Inner:\n    return obj{after}"
                    );
                    let source = format!(
                        "async def f(obj, mode):\n    prepare_scope(obj)\n    try:\n        {}\n    except Error:\n        {}\n    finally:\n        try:\n            publish(obj)\n        except Cleanup as caught:\n            raise Changed from caught\n        except Other:\n            raise\n    return finish(obj)\n",
                        body.replace('\n', "\n        "),
                        handler.replace('\n', "\n        "),
                    );
                    assert_matches_cpython(&source, version);
                }
            }
        }
    }
}

#[test]
fn ordinary_try_iterator_returns_reuse_the_original_finally_pop() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for handler in ["except Error:", "except Error as caught:"] {
            for returned in ["return", "return 'done'"] {
                for polarity in ["is", "is not"] {
                    for calls in 0..3 {
                        let prefix = "                    touch()\n".repeat(calls);
                        let source = format!(
                            "async def f(value, flag, values):\n    await start()\n    try:\n        for item in values:\n            try:\n                if flag:\n{prefix}                    {returned}\n            {handler}\n                pass\n    finally:\n        if value {polarity} None:\n            pass\n"
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn unpacked_or_capture_cleanups_preserve_pop_order_in_protected_scopes() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for leaf in ["value", "str(value)"] {
            for depth in [2, 3] {
                for multiline in [false, true] {
                    let mut pattern = leaf.to_owned();
                    for _ in 0..depth {
                        pattern = if multiline {
                            format!("{{\n    'a': {}\n}}", pattern.replace('\n', "\n    "))
                        } else {
                            format!("{{'a': {pattern}}}")
                        };
                    }
                    for guard in ["", " if check(value)"] {
                        let body = format!(
                            "match obj:\n    case ({} | str(value)){guard}:\n        use(value)\n    case _:\n        pass\nreturn value\n",
                            pattern.replace('\n', "\n    "),
                        );
                        for protected in [false, true] {
                            let body = if protected {
                                format!(
                                    "try:\n    {}\nexcept Error:\n    recover(obj)\n",
                                    body.replace('\n', "\n    ").trim_end()
                                )
                            } else {
                                body.clone()
                            };
                            for definition in ["def", "async def"] {
                                let prefix = if definition == "async def" {
                                    "    await start()\n"
                                } else {
                                    ""
                                };
                                let source = format!(
                                    "{definition} f(obj, value):\n{prefix}    {}",
                                    body.replace('\n', "\n    ")
                                );
                                assert_matches_cpython(&source, version);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn wildcard_normal_try_exits_keep_their_line_before_a_new_setup() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for definition in ["def", "async def"] {
            for wildcard in [
                "fallback(value)",
                "fallback(value=value)",
                "value.fallback()",
            ] {
                for successor in [
                    "try:\n        publish(value)\n    finally:\n        cleanup(value)",
                    "try:\n        if flag:\n            publish(value)\n    finally:\n        cleanup(value)",
                    "try:\n        publish(value)\n    except Other:\n        raise",
                ] {
                    let source = format!(
                        "{definition} f(value, mode, flag):\n    try:\n        match mode:\n            case Constants.FIRST:\n                work(value)\n            case _:\n                {wildcard}\n    except Error:\n        raise\n    {successor}\n",
                    );
                    assert_matches_cpython(&source, version);
                }
            }
        }
    }
}

#[test]
fn ordinary_try_else_guards_keep_their_original_normal_join_nop() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, prefix) in [
            ("def", ""),
            ("def", "    yield value\n"),
            ("async def", "    await start()\n"),
            ("async def", "    await start()\n    yield value\n"),
        ] {
            for condition in [
                "flag",
                "flag is not None",
                "flag or other",
                "flag and other",
                "True",
            ] {
                for recovery in ["pass", "raise Last(value=None) from None"] {
                    for handler_recovery in [
                        "            if other:\n                work()\n",
                        "            raise\n        work(value)\n",
                        "            if other:\n                raise\n            raise\n        work(value)\n",
                    ] {
                        for continuation in ["work(value, other)", "return"] {
                            let source = format!(
                                "{definition} f(value, other, flag):\n{prefix}    try:\n        work()\n    except Error as error:\n        try:\n            work()\n        except Inner as nested:\n{handler_recovery}    else:\n        work(value)\n        if {condition}:\n            try:\n                work(value=value)\n            except Other:\n                {recovery}\n    {continuation}\n",
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn breaks_skip_removed_located_try_exit_markers() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, prefix) in [("def", ""), ("def", "    yield value\n"), ("async def", "")] {
            for condition in ["flag and other", "check() and other < limit"] {
                for handler in ["Error", "(Error, Other)"] {
                    for (cleanup, returned) in [
                        ("pass", "{\"value\": work(value)}"),
                        ("finish()", "None"),
                        ("finish()", "value"),
                        ("finish()", "work()"),
                        ("finish()", "{\"value\": work(value)}"),
                    ] {
                        let source = format!(
                            "{definition} f(value, flag, other, limit):\n{prefix}    try:\n        while {condition}:\n            try:\n                break\n            except {handler}:\n                pass\n    except (Other, Last):\n        pass\n    finally:\n        {cleanup}\n    return {returned}\n",
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn single_constant_arms_preserve_chained_comparison_return_copy_rounds() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for returned in [
            "None",
            "True",
            "False",
            "0",
            "1",
            "256",
            "257",
            "-1",
            "1.5",
            "\"result\"",
            "b\"result\"",
            "...",
        ] {
            for prefix in [
                "",
                "if flag:\n    return None\n",
                "if flag:\n    return None\nif other:\n    return None\n",
            ] {
                for condition in [
                    "low <= value < high",
                    "low <= value < high < limit",
                    "flag and low <= value < high",
                    "low <= obj.value < high",
                ] {
                    let body = format!("{prefix}return {returned} if {condition} else None");
                    let source = format!(
                        "def f(low, value, high, limit, flag, other, obj):\n    try:\n        {}\n    except Error:\n        return None\n",
                        body.replace('\n', "\n        ")
                    );
                    assert_matches_cpython(&source, version);
                }
            }
        }
    }
}

#[test]
fn split_try_ranges_preserve_chained_comparison_return_copies() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for prefix in [
            "",
            "work()\n",
            "value = work()\n",
            "if flag:\n    return None\n",
            "if flag:\n    return None\nvalue = work()\n",
            "if flag:\n    value = work()\nelif other:\n    value = other()\nelse:\n    return None\n",
        ] {
            for condition in [
                "low <= value < high",
                "0 <= value < 100",
                "low <= value < obj.high",
                "low <= obj.value < high",
                "flag and 0 < value < 100",
                "low <= value < high <= limit",
            ] {
                let body = format!("{prefix}return value if {condition} else None");
                let source = format!(
                    "def f(low, value, high, limit, flag, other, obj):\n    try:\n        {}\n    except Error:\n        return None\n",
                    body.replace('\n', "\n        ")
                );
                assert_matches_cpython(&source, version);
            }
        }
    }
}

#[test]
fn handler_finalizer_none_guards_keep_copied_return_owners() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, suspend, consume) in [
            ("def", "", "consume(value)"),
            ("async def", "await ", "consume(value)"),
            ("def", "", "yield value"),
            ("async def", "await ", "yield value"),
        ] {
            for condition in ["flag is None", "flag is not None"] {
                for cleanup in ["work()", "obj.reset(token)", "holder.reset(token)"] {
                    for nested_tail in [false, true] {
                        let tail = if nested_tail {
                            format!(
                                "        token = obj.begin(None)\n        try:\n            {suspend}close(values)\n        finally:\n            obj.reset(token)\n"
                            )
                        } else {
                            String::new()
                        };
                        let source = format!(
                            "{definition} f(obj, flag, values):\n    try:\n        while True:\n            token = obj.begin(flag)\n            try:\n                value = {suspend}advance(values)\n            except StopIteration:\n                return\n            finally:\n                {cleanup}\n            {consume}\n    finally:\n        if {condition}:\n            obj.closed = True\n            obj.value = None\n{tail}"
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn non_final_for_else_breaks_precede_their_copied_backedges() {
    use std::fmt::Write as _;

    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for outer in [None, Some("for item in items:"), Some("while active:")] {
            for condition in [
                "left or right",
                "left and right",
                "not (low <= value <= high)",
                "flag",
            ] {
                for otherwise in ["return False", "work(value)", "raise Error()"] {
                    for has_else in [false, true] {
                        let mut body = format!(
                            "for value in values:\n    if flag:\n        if {condition}:\n            break\n"
                        );
                        if has_else {
                            body.push_str(
                                "    else:\n        work(value)\n        if other:\n            break\n",
                            );
                        }
                        writeln!(body, "else:\n    {otherwise}").unwrap();
                        if let Some(outer) = outer {
                            body = format!(
                                "{outer}\n    {}\n",
                                body.trim_end().replace('\n', "\n    ")
                            );
                        }
                        let source = format!(
                            "def f(items, values, value, flag, other, left, right, low, high, active):\n    {}\n    return True\n",
                            body.trim_end().replace('\n', "\n    "),
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn protected_for_else_branch_exits_keep_their_original_order() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, prefix) in [("def", ""), ("async def", ""), ("def", "    yield value\n")] {
            for condition in [
                "left or right",
                "left and right",
                "not (low <= value <= high)",
                "flag",
            ] {
                for protection in ["none", "except", "finally", "with"] {
                    let mut body = format!(
                        "for value in values:\n    if flag:\n        if {condition}:\n            break\n    else:\n        work(value)\n        if other:\n            break\nelse:\n    return\n"
                    );
                    body = match protection {
                        "except" => format!(
                            "try:\n    {}\nexcept Error:\n    recover()\n",
                            body.trim_end().replace('\n', "\n    "),
                        ),
                        "finally" => format!(
                            "try:\n    {}\nfinally:\n    cleanup()\n",
                            body.trim_end().replace('\n', "\n    "),
                        ),
                        "with" => format!(
                            "with manager:\n    {}\n",
                            body.trim_end().replace('\n', "\n    "),
                        ),
                        _ => body,
                    };
                    let source = format!(
                        "{definition} f(values, value, flag, other, left, right, low, high, manager):\n{prefix}    {}\n    return\n",
                        body.trim_end().replace('\n', "\n    "),
                    );
                    assert_matches_cpython(&source, version);
                }
            }
        }
    }
}

#[test]
fn async_for_else_branch_exits_keep_their_original_order() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for prefix in ["", "yield value\n"] {
            for condition in [
                "left or right",
                "left and right",
                "not (low <= value <= high)",
                "flag",
            ] {
                for branch in ["break", "return", "work(value)"] {
                    for has_else in [false, true] {
                        let otherwise = if has_else {
                            "else:\n    raise Error()\n"
                        } else {
                            ""
                        };
                        let body = format!(
                            "{prefix}async for value in values:\n    if flag:\n        if {condition}:\n            {branch}\n    else:\n        work(value)\n        if other:\n            break\n{otherwise}finish()\nreturn"
                        );
                        let source = format!(
                            "async def f(values, value, flag, other, left, right, low, high):\n    {}\n",
                            body.replace('\n', "\n    "),
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn handler_break_footers_count_original_exception_pops() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, prefix) in [("def", ""), ("def", "    yield value\n"), ("async def", "")] {
            for handler in ["unnamed", "named", "nested"] {
                for returned in ["value", "None", "work()"] {
                    for exit in ["break", "if flag:\n    break"] {
                        let mut body = format!("with manager:\n    cleanup()\n{exit}");
                        if handler == "nested" {
                            body = format!(
                                "try:\n    work()\nexcept Inner:\n    {}",
                                body.replace('\n', "\n    ")
                            );
                        }
                        let binding = if handler == "named" { " as error" } else { "" };
                        let source = format!(
                            "{definition} f(items, manager, flag, value):\n{prefix}    for item in items:\n        try:\n            work()\n        except Error{binding}:\n            {}\n        work()\n    return {returned}\n",
                            body.replace('\n', "\n            ")
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn cold_recovery_continuations_keep_the_terminal_if_footer() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, prefix) in [("def", ""), ("def", "    yield value\n"), ("async def", "")] {
            for protected in ["work()", "raise Error"] {
                for recovery in ["pass", "recover()"] {
                    for otherwise in [
                        "",
                        "        else:\n            raise Other\n",
                        "        else:\n            work()\n",
                    ] {
                        for suffix in ["work()", "assert other"] {
                            let source = format!(
                                "{definition} f(flag, other, value):\n{prefix}    if flag:\n        try:\n            {protected}\n        except Error:\n            {recovery}\n{otherwise}        {suffix}\n"
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn manager_comprehension_continuations_keep_the_terminal_if_footer() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for manager in ["with", "async with"] {
            for expression in [
                "value = [item async for item in items]",
                "assert [item async for item in items] == []",
                "assert other and [item async for item in items]",
                "assert other or [item async for item in items]",
                "value = [item for item in items]",
            ] {
                for suffix in ["work()", "await work()", "assert not await work()"] {
                    let source = format!(
                        "async def f(flag, other, items, manager):\n    if flag:\n        {manager} manager:\n            {expression}\n        {suffix}\n"
                    );
                    assert_matches_cpython(&source, version);
                }
            }
        }
    }
}

#[test]
fn negated_final_elif_keeps_chained_comparison_backedges() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for loop_header in ["for value in values", "while active"] {
            for condition in [
                "not (flag or low <= value <= high)",
                "not (value in values or 1 <= value <= 5 or 8 <= value <= 12)",
            ] {
                for ending in [
                    "return None",
                    "return value",
                    "raise Error()",
                    "continue",
                    "work()",
                ] {
                    let source = format!(
                        "def f(values, value, flag, low, high, active):\n    {loop_header}:\n        if value == 0:\n            work(value)\n        elif {condition}:\n            {ending}\n    return None\n"
                    );
                    assert_matches_cpython(&source, version);
                }
            }
        }
    }
}

#[test]
fn implicit_generator_owner_keeps_final_chain_backedges() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, prefix) in [
            ("async def", ""),
            ("def", "    yield value\n"),
            ("async def", "    yield value\n"),
        ] {
            for loop_header in ["for value in values", "while active"] {
                for condition in [
                    "not (flag or low <= value <= high)",
                    "not (value in values or 1 <= value <= 5 or 8 <= value <= 12)",
                ] {
                    for ending in ["return", "raise Error()", "continue", "work()"] {
                        let source = format!(
                            "{definition} f(values, value, flag, low, high, active):\n{prefix}    {loop_header}:\n        if value == 0:\n            work(value)\n        elif {condition}:\n            {ending}\n    return\n"
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn deferred_comparisons_do_not_change_enclosing_loop_backedges() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, prefix) in [
            ("def", ""),
            ("async def", ""),
            ("def", "    yield value\n"),
            ("async def", "    yield value\n"),
        ] {
            for loop_header in ["for value in values", "while active"] {
                for value in [
                    "any(low <= item <= high for item in values)",
                    "any(item for item in values if low <= item <= high)",
                    "(lambda: low <= value <= high)()",
                ] {
                    for operator in ["and", "or"] {
                        for ending in ["return", "work(value)"] {
                            let source = format!(
                                "{definition} f(values, value, flag, other, low, high, active):\n{prefix}    {loop_header}:\n        if flag:\n            work(value)\n        elif other {operator} {value}:\n            {ending}\n    return\n"
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn guarded_raising_recovery_keeps_the_normal_manager_join_owned() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, protected) in [
            ("def", "work()"),
            ("async def", "await work()"),
            ("def", "yield value"),
            ("async def", "yield value"),
        ] {
            for call in ["work(value)", "work({key: item for key, item in items})"] {
                for binding in ["", " as error"] {
                    for raised in ["raise", "raise Other()"] {
                        let source = format!(
                            "{definition} f(value, items, manager, flag):\n    with manager:\n        try:\n            {protected}\n        finally:\n            if flag:\n                try:\n                    {call}\n                except Error{binding}:\n                    recover()\n                    {raised}\n"
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn shared_recovery_exits_do_not_supply_a_unique_finalizer_line() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, prefix, returned, cleanup) in [
            ("def", "", "return value", "cleanup()"),
            ("def", "    yield value\n", "return value", "cleanup()"),
            ("async def", "", "return await work()", "await cleanup()"),
        ] {
            for binding in ["", " as error"] {
                for ending in ["except CleanupError:\n    pass", "finally:\n    finish()"] {
                    let source = format!(
                        "{definition} f(value):\n{prefix}    try:\n        {returned}\n    except Error{binding}:\n        pass\n    except Other:\n        recover()\n    finally:\n        try:\n            {cleanup}\n        {}\n    after()\n",
                        ending.replace('\n', "\n        ")
                    );
                    assert_matches_cpython(&source, version);
                }
            }
        }
    }
}

#[test]
fn pass_recovery_keeps_its_line_before_a_nested_finalizer() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, prefix, returned, cleanup) in [
            ("def", "", "return value", "cleanup()"),
            ("def", "    yield value\n", "return value", "cleanup()"),
            ("async def", "", "return await work()", "await cleanup()"),
        ] {
            for binding in ["", " as error"] {
                for (earlier, later) in [
                    ("", ""),
                    ("except Other:\n    raise\n", ""),
                    ("", "except Other:\n    raise\n"),
                ] {
                    for ending in ["except CleanupError:\n    pass", "finally:\n    finish()"] {
                        for manager in [false, true] {
                            let mut body = format!(
                                "try:\n    {returned}\n{earlier}except Error{binding}:\n    pass\n{later}finally:\n    try:\n        {cleanup}\n    {}\nafter()",
                                ending.replace('\n', "\n    ")
                            );
                            if manager {
                                body =
                                    format!("with resource:\n    {}", body.replace('\n', "\n    "));
                            }
                            let source = format!(
                                "{definition} f(value, resource):\n{prefix}    {}\n",
                                body.replace('\n', "\n    ")
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn structured_nested_finalizers_keep_their_outer_guard_line() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, prefix, work, cleanup) in [
            ("def", "", "work()", "cleanup()"),
            ("def", "    yield result\n", "work()", "cleanup()"),
            ("async def", "", "await work()", "await cleanup()"),
        ] {
            for guard in ["flag", "flag and other", "flag or other"] {
                for finalizer in [
                    format!("for item in items:\n    if item:\n        {cleanup}"),
                    format!("try:\n    {cleanup}\nfinally:\n    close()"),
                ] {
                    for handler in ["", "except Error:\n    recover()\n"] {
                        for follow in ["after()", "return result"] {
                            let nested = format!(
                                "try:\n    {work}\n{handler}finally:\n    {}",
                                finalizer.replace('\n', "\n    ")
                            );
                            let source = format!(
                                "{definition} f(flag, other, items, result):\n{prefix}    try:\n        work()\n    finally:\n        if {guard}:\n            {}\n    {follow}\n",
                                nested.replace('\n', "\n            ")
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn manager_cleanup_jump_keeps_its_saved_owner_before_guard_join() {
    let oracle = r"
import ast, dis, importlib.util, marshal, sys, types
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
source = sys.argv[3]
tree = ast.parse(source)
guard_line = next(node.lineno for node in ast.walk(tree)
                  if isinstance(node, ast.If)
                  and any(isinstance(child, ast.Try) for child in node.body))
loop_line = next(node.lineno for node in ast.walk(tree) if isinstance(node, ast.For))
def cleanup_owner(module):
    code = next(value for value in module.co_consts if isinstance(value, types.CodeType))
    instructions = list(dis.get_instructions(code))
    guard = next(item for item in instructions
                 if item.positions.lineno == guard_line and item.opname == 'POP_JUMP_IF_FALSE')
    jump = next(item for item in reversed(instructions)
                if item.offset < guard.argval and item.opname == 'JUMP_FORWARD'
                and item.positions.lineno == loop_line)
    owner = next((entry for entry in dis.Bytecode(code).exception_entries
                  if entry.start <= jump.offset < entry.end), None)
    if owner is None:
        return None
    handler = next(index for index, item in enumerate(instructions) if item.offset == owner.target)
    return owner.depth, owner.lasti, tuple(item.opname for item in instructions[handler:handler + 3])
expected = cleanup_owner(compile(source, 'finally_loop_exit_locations.py', 'exec', dont_inherit=True))
actual = cleanup_owner(marshal.loads(sys.stdin.buffer.read()))
assert expected is not None and 'WITH_EXCEPT_START' in expected[2]
assert actual == expected, (actual, expected)
";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        let target = version.target();
        let (variable, fallback) = match version {
            PythonVersion::Py314 => ("SERC_TEST_PYTHON", "python3.14"),
            PythonVersion::Py315 => ("SERC_TEST_PYTHON_315", "python3.15"),
            _ => unreachable!(),
        };
        let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for (definition, prefix, work, cleanup) in [
            ("def", "", "work()", "cleanup()"),
            ("def", "yield result\n", "work()", "cleanup()"),
            ("async def", "", "await work()", "await cleanup()"),
        ] {
            for guard in ["flag", "flag or other"] {
                for protected in [false, true] {
                    let mut body = format!(
                        "with manager:\n    try:\n        work()\n    finally:\n        if {guard}:\n            try:\n                {work}\n            finally:\n                for value in values:\n                    if value:\n                        {cleanup}\n"
                    );
                    if protected {
                        body = format!(
                            "try:\n    {}\nexcept OuterError:\n    recover()\n",
                            body.trim_end().replace('\n', "\n    ")
                        );
                    }
                    body.insert_str(0, prefix);
                    let source = format!(
                        "{definition} f(flag, other, values, result, manager):\n    {}\n    after()\n",
                        body.trim_end().replace('\n', "\n    ")
                    );
                    // The missing guard marker is a separate layout issue. The
                    // preceding jump must retain its native manager owner meanwhile.
                    for compiled in [
                        compile_with_options(&source, "finally_loop_exit_locations.py", &options)
                            .unwrap(),
                        compile_bytes_with_options(
                            source.as_bytes(),
                            "finally_loop_exit_locations.py",
                            &options,
                        )
                        .unwrap(),
                    ] {
                        let mut child = Command::new(&python)
                            .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle])
                            .args([
                                target.tag.trim_start_matches('v'),
                                &target.magic_hex(),
                                &source,
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
                            .write_all(&compiled.marshal())
                            .unwrap();
                        let output = child.wait_with_output().unwrap();
                        assert!(
                            output.status.success(),
                            "{version:?}: {source}\n{}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn normal_finally_loop_exits_retain_direct_small_return_locations() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for mode in ["def", "async def"] {
            for condition in ["flag", "not work()", "flag and other", "flag or other"] {
                for body in ["work()", "if work():\n    work()"] {
                    for prefix in ["", "for value in values:\n    work(value)\n"] {
                        for follow in [
                            "return result",
                            "return None",
                            "work()\nreturn result",
                            "result = work() if flag else other\nreturn result",
                            "if flag:\n    work()\nreturn result",
                            "return work()",
                            "return result + other",
                        ] {
                            // Larger continuations after a guarded body have independent
                            // borrow-boundary coverage; this matrix varies direct exit copying.
                            if body.starts_with("if") && !follow.starts_with("return") {
                                continue;
                            }
                            let cleanup = format!(
                                "{prefix}while {condition}:\n    {}",
                                body.replace('\n', "\n    ")
                            );
                            let source = format!(
                                "{mode} f(values, flag, other, result):\n    try:\n        work()\n    finally:\n        {}\n    {}\n",
                                cleanup.replace('\n', "\n        "),
                                follow.replace('\n', "\n    ")
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn earlier_empty_loop_arrays_do_not_drop_later_exit_locations() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for protected in [
            "work()",
            "await work()",
            "for item in values:\n    await item\n    if flag:\n        work()",
        ] {
            for prefix in [
                "for item in values:\n    work()",
                "for item in values:\n    if flag:\n        work()",
            ] {
                for condition in ["flag", "not result.empty()"] {
                    for follow in [
                        "return result",
                        "work()\nreturn result",
                        "result['status'] = 'complete' if result['ok'] == len(values) else 'failed'\nreturn dict(result)",
                    ] {
                        let cleanup = format!("{prefix}\nwhile {condition}:\n    work()");
                        let source = format!(
                            "async def f(values, flag, other, result):\n    try:\n        {}\n    finally:\n        {}\n    {}\n",
                            protected.replace('\n', "\n        "),
                            cleanup.replace('\n', "\n        "),
                            follow.replace('\n', "\n    ")
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn earlier_branch_exits_share_the_final_else_loop_continuation() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for managed in [false, true] {
            for first in [
                "work()",
                "return result",
                "raise Error()",
                "if last:\n    return result",
            ] {
                for condition in [
                    "other",
                    "not work()",
                    "other is None",
                    "other and last",
                    "other or last",
                ] {
                    for body in ["pass", "work()", "if last:\n    break"] {
                        for otherwise in ["", "\nelse:\n    work()"] {
                            let loop_body = format!(
                                "while {condition}:\n    {}{otherwise}",
                                body.replace('\n', "\n    ")
                            );
                            let protected = format!(
                                "if flag:\n    {}\nelse:\n    {}",
                                first.replace('\n', "\n    "),
                                loop_body.replace('\n', "\n    ")
                            );
                            let mut source = format!(
                                "try:\n    {}\nfinally:\n    cleanup()",
                                protected.replace('\n', "\n    ")
                            );
                            if managed {
                                source = format!(
                                    "with manager:\n    {}",
                                    source.replace('\n', "\n    ")
                                );
                            }
                            let source = format!(
                                "def f(manager, flag, other, last, result):\n    {}\n",
                                source.replace('\n', "\n    ")
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn finalizer_iterator_break_joins_keep_their_normal_exhaustion_line() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for mode in ["def", "async def"] {
            let work = if mode == "async def" {
                "await work()"
            } else {
                "work()"
            };
            for scope in ["plain", "with", "try"] {
                let continuations: &[&str] = if scope == "try" {
                    &["return result", "return None"]
                } else {
                    &[
                        "return result",
                        "return None",
                        "assert flag",
                        "assert flag and other",
                        "work()\nreturn result",
                    ]
                };
                for body in [
                    format!("if flag:\n    break\n{work}"),
                    format!("{work}\nif flag:\n    break"),
                    work.to_owned(),
                ] {
                    for follow in continuations {
                        for prefix in ["", "cleanup()\n"] {
                            let cleanup = format!(
                                "{prefix}for item in values:\n    {}",
                                body.replace('\n', "\n    ")
                            );
                            let mut source = format!(
                                "try:\n    {work}\nfinally:\n    {}\n{follow}",
                                cleanup.replace('\n', "\n    ")
                            );
                            if scope == "with" {
                                source = format!(
                                    "with manager:\n    {}",
                                    source.replace('\n', "\n    ")
                                );
                            } else if scope == "try" {
                                source = format!(
                                    "try:\n    {}\nexcept Error:\n    work()",
                                    source.replace('\n', "\n    ")
                                );
                            }
                            let source = format!(
                                "{mode} f(manager, values, flag, other, result):\n    {}\n",
                                source.replace('\n', "\n    ")
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn final_else_work_keeps_its_join_after_structured_earlier_cleanup() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for mode in ["def", "generator", "async def", "async generator"] {
            let suspended = match mode {
                "generator" => "for item in values:\n    yield item",
                "async def" | "async generator" => "await work()",
                _ => "work()",
            };
            for manager_count in 0..=2 {
                for protected in [
                    "work()",
                    "for item in values:\n    work()",
                    "if other:\n    work()",
                    suspended,
                ] {
                    for earlier in [
                        "cleanup()",
                        "if other:\n    cleanup()",
                        "with manager:\n    cleanup()",
                        "return",
                        "raise Error()",
                    ] {
                        for follow in ["", "return result", "work()\nreturn result"] {
                            let follow = if mode == "async generator" {
                                match follow {
                                    "return result" => "return",
                                    "work()\nreturn result" => "work()\nreturn",
                                    _ => follow,
                                }
                            } else {
                                follow
                            };
                            let mut source = format!(
                                "try:\n    {}\nfinally:\n    if flag:\n        {}\n    else:\n        cleanup()",
                                protected.replace('\n', "\n    "),
                                earlier.replace('\n', "\n        ")
                            );
                            if !follow.is_empty() {
                                source.push('\n');
                                source.push_str(follow);
                            }
                            for _ in 0..manager_count {
                                source = format!(
                                    "with manager:\n    {}",
                                    source.replace('\n', "\n    ")
                                );
                            }
                            let definition = if mode.ends_with("generator") {
                                source = format!("yield result\n{source}");
                                if mode == "async generator" {
                                    "async def"
                                } else {
                                    "def"
                                }
                            } else {
                                mode
                            };
                            let source = format!(
                                "{definition} f(manager, values, flag, other, result):\n    {}\n",
                                source.replace('\n', "\n    ")
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn copied_return_cleanup_does_not_own_the_normal_final_else_exit() {
    let oracle = r"
import dis, importlib.util, marshal, sys, types
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]

def normal_else_exit(code):
    function = next(value for value in code.co_consts
                    if isinstance(value, types.CodeType) and value.co_name == 'f')
    instructions = list(dis.get_instructions(function))
    branch = next(instruction for instruction in instructions
                  if instruction.opname == 'POP_JUMP_IF_FALSE')
    start = next(index for index, instruction in enumerate(instructions)
                 if instruction.offset == branch.argval)
    assert instructions[start].opname == 'LOAD_GLOBAL'
    assert instructions[start].argval == 'finish'
    route = instructions[start:start + 4]
    assert [instruction.opname for instruction in route] == [
        'LOAD_GLOBAL', 'CALL', 'POP_TOP', 'NOP']
    marker = route[-1]
    owner = next((entry for entry in dis.Bytecode(function).exception_entries
                  if entry.start <= marker.offset < entry.end), None)
    return [(instruction.opname, instruction.arg, tuple(instruction.positions))
            for instruction in route], owner

expected, expected_owner = normal_else_exit(compile(
    sys.argv[3], 'finally_loop_exit_locations.py', 'exec', dont_inherit=True, optimize=0))
actual, actual_owner = normal_else_exit(marshal.loads(sys.stdin.buffer.read()))
assert actual == expected, (actual, expected)
assert expected_owner is None, expected_owner
assert actual_owner is None, actual_owner
";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        let target = version.target();
        let (variable, fallback) = match version {
            PythonVersion::Py314 => ("SERC_TEST_PYTHON", "python3.14"),
            PythonVersion::Py315 => ("SERC_TEST_PYTHON_315", "python3.15"),
            _ => unreachable!(),
        };
        let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for (definition, prefix, work) in [
            ("def", "", "work()"),
            ("def", "    yield result\n", "work()"),
            ("async def", "", "await work()"),
        ] {
            let source = format!(
                "{definition} f(manager, flag, result):
{prefix}    with manager:
        try:
            {work}
        finally:
            if flag:
                try:
                    return
                finally:
                    cleanup()
            else:
                finish()
"
            );
            // Follow the normal false edge; copied exception cleanup has an
            // independent layout difference and cannot serve as this owner oracle.
            for compiled in [
                compile_with_options(&source, "finally_loop_exit_locations.py", &options).unwrap(),
                compile_bytes_with_options(
                    source.as_bytes(),
                    "finally_loop_exit_locations.py",
                    &options,
                )
                .unwrap(),
            ] {
                let mut child = Command::new(&python)
                    .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle])
                    .args([
                        target.tag.trim_start_matches('v'),
                        &target.magic_hex(),
                        &source,
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
                    .write_all(&compiled.marshal())
                    .unwrap();
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "{version:?}: {source}\n{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}

#[test]
fn closed_final_else_assertions_keep_their_unique_success_donor() {
    fn indent(source: &str) -> String {
        let mut indented = String::new();
        for line in source.lines() {
            indented.push_str("    ");
            indented.push_str(line);
            indented.push('\n');
        }
        indented
    }

    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for definition in ["def", "async def"] {
            for condition in [
                "a and b",
                "a is not None and b is not None",
                "a == b and b == work(a)",
                "not a",
                "a < b < value",
            ] {
                for earlier in [
                    "if other:\n    work(a)\n",
                    "if other:\n    work(a)\nelif value:\n    work(b)\n",
                    "if other:\n    return a\nelif value:\n    return b\n",
                ] {
                    for context in ["branch", "try", "loop", "finally"] {
                        let mut inner = format!("{earlier}else:\n    assert {condition}\n");
                        if context == "try" {
                            inner =
                                format!("try:\n{}except Error:\n    recover(a)\n", indent(&inner));
                        }
                        let mut body =
                            format!("if flag:\n{}else:\n    fallback(b)\n", indent(&inner));
                        body = match context {
                            "loop" => format!("for item in values:\n{}", indent(&body)),
                            "finally" => {
                                format!("try:\n    prepare(a)\nfinally:\n{}", indent(&body))
                            }
                            _ => body,
                        };
                        body.push_str("after(a, b)\n");
                        let source = format!(
                            "{definition} f(a, b, value, values, flag, other):\n{}",
                            indent(&body),
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn assertion_success_donates_locations_to_exception_group_cleanup_copies() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for expression in ["a and b", "(\n    a\n    and b\n)"] {
            for continuation in ["after(a, b)", "raise Error()", "raise Error(a)"] {
                let assertion = format!("assert {expression}").replace('\n', "\n                ");
                let source = format!(
                    "def f(a, b, value, flag, other):\n    try:\n        prepare(a)\n    except* Error:\n        if flag:\n            if other:\n                work(a)\n            elif value:\n                work(b)\n            else:\n                {assertion}\n        else:\n            fallback(b)\n    {continuation}\n"
                );
                assert_matches_cpython(&source, version);
            }
        }
    }
}

#[test]
fn assertion_success_retains_its_location_through_outer_branch_joins() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for mode in ["def", "generator", "async def"] {
            for assertion in [
                "assert flag",
                "assert not flag",
                "assert flag and other",
                "assert flag\nassert other",
                "assert work()",
            ] {
                for prefix in ["", "cleanup()\n"] {
                    for returned in ["result", "None", "{}"] {
                        for nested in [false, true] {
                            let work = if mode == "async def" {
                                "await work()"
                            } else {
                                "work()"
                            };
                            let mut body = format!(
                                "try:\n    {work}\nfinally:\n    {}",
                                format!("{prefix}{assertion}").replace('\n', "\n    ")
                            );
                            body = format!(
                                "if first:\n    {}\nelif other:\n    raise Error()",
                                body.replace('\n', "\n    ")
                            );
                            if nested {
                                body = format!(
                                    "if other:\n    {}\nelif first:\n    raise Error()",
                                    body.replace('\n', "\n    ")
                                );
                            }
                            let definition = if mode == "generator" {
                                body = format!("yield result\n{body}");
                                "def"
                            } else {
                                mode
                            };
                            let source = format!(
                                "{definition} f(first, flag, other, result):\n    {}\n    return {returned}\n",
                                body.replace('\n', "\n    ")
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn uncertified_return_chains_do_not_add_an_assertion_line_slot() {
    let oracle = r"
import dis, importlib.util, marshal, sys, types
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]

def success_has_nop(code):
    function = next(value for value in code.co_consts
                    if isinstance(value, types.CodeType) and value.co_name == 'f')
    instructions = list(dis.get_instructions(function))
    success = next(instruction for instruction in instructions
                   if instruction.opname == 'POP_JUMP_IF_TRUE')
    target = next(instruction for instruction in instructions
                  if instruction.offset == success.argval)
    return target.opname == 'NOP'

expected = success_has_nop(compile(
    sys.argv[3], 'finally_loop_exit_locations.py', 'exec', dont_inherit=True, optimize=0))
actual = success_has_nop(marshal.loads(sys.stdin.buffer.read()))
assert not expected
assert actual == expected, (actual, expected)
";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        let target = version.target();
        let (variable, fallback) = match version {
            PythonVersion::Py314 => ("SERC_TEST_PYTHON", "python3.14"),
            PythonVersion::Py315 => ("SERC_TEST_PYTHON_315", "python3.15"),
            _ => unreachable!(),
        };
        let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for (definition, prefix, work) in [
            ("def", "", "work()"),
            ("def", "    yield result\n", "work()"),
            ("async def", "", "await work()"),
        ] {
            for (returned, depth) in [
                ("result, other", 2),
                ("work()", 2),
                ("result", 6),
                ("None", 5),
                ("{}", 6),
            ] {
                let mut body = format!("try:\n    {work}\nfinally:\n    assert flag");
                for _ in 0..depth {
                    body = format!(
                        "if first:\n    {}\nelif other:\n    raise Error()",
                        body.replace('\n', "\n    ")
                    );
                }
                let source = format!(
                    "{definition} f(first, flag, other, result):\n{prefix}    {}\n    return {returned}\n",
                    body.replace('\n', "\n    ")
                );
                // A later return-copy layout difference must not authorize a
                // new NOP on this original assertion-success edge.
                for compiled in [
                    compile_with_options(&source, "finally_loop_exit_locations.py", &options)
                        .unwrap(),
                    compile_bytes_with_options(
                        source.as_bytes(),
                        "finally_loop_exit_locations.py",
                        &options,
                    )
                    .unwrap(),
                ] {
                    let mut child = Command::new(&python)
                        .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle])
                        .args([
                            target.tag.trim_start_matches('v'),
                            &target.magic_hex(),
                            &source,
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
                        .write_all(&compiled.marshal())
                        .unwrap();
                    let output = child.wait_with_output().unwrap();
                    assert!(
                        output.status.success(),
                        "{version:?}: {source}\n{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
        }
    }
}

#[test]
fn post_manager_breaks_keep_only_originally_located_jump_markers() {
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for (definition, prefix, manager, work) in [
            ("def", "", "with", "work()"),
            ("def", "yield value\n", "with", "work()"),
            ("async def", "", "with", "await work()"),
            ("async def", "", "async with", "await work()"),
        ] {
            for cleanup in [
                None,
                Some("pass"),
                Some("cleanup()"),
                Some("other = value"),
                Some("del value"),
                Some("cleanup()\ncleanup()"),
            ] {
                for returned in ["value", "value, flag, items", "None"] {
                    let protected = format!("{manager} manager:\n    {work}\nbreak");
                    let body = cleanup.map_or_else(
                        || protected.clone(),
                        |cleanup| {
                            format!(
                                "try:\n    {}\nfinally:\n    {}",
                                protected.replace('\n', "\n    "),
                                cleanup.replace('\n', "\n    ")
                            )
                        },
                    );
                    let body = format!(
                        "{prefix}while flag:\n    {}\nreturn {returned}",
                        body.replace('\n', "\n    ")
                    );
                    let source = format!(
                        "{definition} f(flag, manager, value, items):\n    {}\n",
                        body.replace('\n', "\n    ")
                    );
                    assert_matches_cpython(&source, version);
                }
            }
        }
    }
}

#[test]
fn tuple_prefixes_keep_outer_owners_across_retained_finally_setups() {
    use std::fmt::Write as _;

    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, suspension, generator) in [
            ("def", "", false),
            ("async def", "await ", false),
            ("def", "", true),
            ("async def", "await ", true),
        ] {
            for (pairs, attribute) in [(3, false), (3, true), (4, true), (6, true)] {
                for joined in [false, true] {
                    for trailing in [false, true] {
                        let mut source = format!("{definition} f(obj, flag):\n");
                        if joined {
                            source.push_str("    if flag:\n        work()\n");
                        }
                        for index in 0..pairs {
                            if attribute {
                                writeln!(
                                    source,
                                    "    value{index}, obj.item{index} = obj.item{index}, None"
                                )
                                .unwrap();
                            } else {
                                writeln!(source, "    value{index}, flag = flag, None").unwrap();
                            }
                        }
                        if trailing {
                            source.push_str("    obj.last = None\n");
                        }
                        let last = pairs - 1;
                        write!(source, "    try:\n        if value{last} is not None:\n            {suspension}work(value{last})\n    finally:\n        if flag is not None:\n            {suspension}cleanup(flag)\n").unwrap();
                        if generator {
                            source.push_str("    yield flag\n");
                        }
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn folded_manager_exits_keep_originally_shared_finally_pops_unlocated() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, prefix, manager, work) in [
            ("def", "", "with", "work()"),
            ("def", "yield value\n", "with", "work()"),
            ("async def", "", "with", "await work()"),
            ("async def", "", "async with", "await work()"),
        ] {
            for protected in [
                work.to_owned(),
                "return value".to_owned(),
                "raise Error()".to_owned(),
                format!(
                    "while True:\n    {work}\n    if flag:\n        return value\n    else:\n        raise Error()"
                ),
            ] {
                for cleanup in ["cleanup()", "if flag:\n    cleanup()\nelse:\n    cleanup()"] {
                    let body = format!(
                        "{prefix}try:\n    {manager} manager:\n        {}\nfinally:\n    try:\n        {}\n    finally:\n        cleanup()",
                        protected.replace('\n', "\n        "),
                        cleanup.replace('\n', "\n        ")
                    );
                    let source = format!(
                        "{definition} f(flag, manager, value):\n    {}\n",
                        body.replace('\n', "\n    ")
                    );
                    assert_matches_cpython(&source, version);
                }
            }
        }
    }
}

#[test]
fn folded_handler_breaks_keep_copied_cleanup_jumps_interruptible() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, generator, work) in [
            ("def", false, "work()"),
            ("def", true, "work()"),
            ("async def", false, "await work()"),
            ("async def", true, "await work()"),
        ] {
            for binding in ["", " as error"] {
                for recovery in [
                    format!("while True:\n    {work}\n    if flag:\n        break"),
                    format!(
                        "while True:\n    try:\n        {work}\n    except Inner:\n        continue\n    break"
                    ),
                ] {
                    for suffix in [
                        "finish()\nfinish()",
                        "for item in items:\n    finish(item)",
                        "return",
                    ] {
                        if version == PythonVersion::Py314 && suffix.starts_with("for ") {
                            continue;
                        }
                        let prefix = if generator { "yield value\n" } else { "" };
                        let body = format!(
                            "{prefix}try:\n    {work}\nexcept Error{binding}:\n    {}\n{suffix}",
                            recovery.replace('\n', "\n    ")
                        );
                        let source = format!(
                            "{definition} f(flag, items, value):\n    {}\n",
                            body.replace('\n', "\n    ")
                        );
                        assert_matches_cpython(&source, version);
                    }
                }
            }
        }
    }
}

#[test]
fn coroutine_named_handler_break_none_markers_reuse_name_cleanup_owners() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for loop_header in ["while flag:", "for item in items:"] {
            for outer_try in [false, true] {
                for prefix in [
                    "result = None",
                    "await recover(values)",
                    "if values:\n    await recover(values)",
                ] {
                    for condition in ["flag is None", "flag is not None"] {
                        for cleanup in [
                            format!("if {condition}:\n    work(flag)"),
                            format!("(work if {condition} else finish)(flag)"),
                        ] {
                            let mut protected = format!(
                                "try:\n    value = await advance(values)\nexcept Error as error:\n    {}\n    break",
                                prefix.replace('\n', "\n    ")
                            );
                            if outer_try {
                                protected =
                                    format!("try:\n    {}", protected.replace('\n', "\n    "));
                            }
                            let body = format!(
                                "{protected}\nfinally:\n    {}",
                                cleanup.replace('\n', "\n    ")
                            );
                            let source = format!(
                                "async def f(items, values, flag):\n    {loop_header}\n        {}\n",
                                body.replace('\n', "\n        ")
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn nested_finally_raise_guards_share_the_outer_pop_line_event() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        for (definition, work) in [
            ("def", "work(obj)"),
            ("def", "yield obj"),
            ("async def", "await work(obj)"),
            ("async def", "yield obj"),
        ] {
            for prefix in [
                "",
                "audit(error)\n",
                "try:\n    recover(obj)\nexcept Error as caught:\n    if error is None:\n        error = caught\n",
            ] {
                for condition in ["error is not None", "error"] {
                    for raised in ["error", "Error(error)"] {
                        for cleanup in ["finish()", "obj.finish()"] {
                            let finalizer = format!("{prefix}if {condition}:\n    raise {raised}");
                            let source = format!(
                                "{definition} f(error, obj, values):\n    try:\n        try:\n            {work}\n        finally:\n            {}\n    finally:\n        {cleanup}\n",
                                finalizer.replace('\n', "\n            ")
                            );
                            assert_matches_cpython(&source, version);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn folded_handler_breaks_keep_exact_cleanup_layout_before_py314_iterators() {
    let oracle = r"
import dis, importlib.util, marshal, sys, types
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]

def function(code):
    return next(value for value in code.co_consts
                if isinstance(value, types.CodeType) and value.co_name == 'f')

expected = function(compile(sys.argv[3], 'finally_loop_exit_locations.py', 'exec',
                            dont_inherit=True, optimize=0))
actual = function(marshal.loads(sys.stdin.buffer.read()))
# The warm iterator still has an independent borrowed-load mismatch on 3.14.
# The copied cleanup's opcode, offsets, positions, and owners must be exact.
def cold_cleanup(code):
    instructions = list(dis.get_instructions(code, show_caches=True))
    start = next(index for index, item in enumerate(instructions)
                 if item.opname == 'PUSH_EXC_INFO')
    return [(item.offset, item.opname, item.arg, tuple(item.positions))
            for item in instructions[start:]]

assert cold_cleanup(actual) == cold_cleanup(expected), (cold_cleanup(actual), cold_cleanup(expected))
assert actual.co_linetable == expected.co_linetable
assert actual.co_exceptiontable == expected.co_exceptiontable
assert list(actual.co_positions()) == list(expected.co_positions())
";
    let version = PythonVersion::Py314;
    let target = version.target();
    let python = std::env::var_os("SERC_TEST_PYTHON").unwrap_or_else(|| "python3.14".into());
    let options = CompileOptions {
        python_version: version,
        ..CompileOptions::default()
    };
    for (definition, generator, work) in [
        ("def", false, "work()"),
        ("def", true, "work()"),
        ("async def", false, "await work()"),
        ("async def", true, "await work()"),
    ] {
        for binding in ["", " as error"] {
            for recovery in [
                format!("while True:\n    {work}\n    if flag:\n        break"),
                format!(
                    "while True:\n    try:\n        {work}\n    except Inner:\n        continue\n    break"
                ),
            ] {
                let prefix = if generator { "yield value\n" } else { "" };
                let body = format!(
                    "{prefix}try:\n    {work}\nexcept Error{binding}:\n    {}\nfor item in items:\n    finish(item)",
                    recovery.replace('\n', "\n    ")
                );
                let source = format!(
                    "{definition} f(flag, items, value):\n    {}\n",
                    body.replace('\n', "\n    ")
                );
                for compiled in [
                    compile_with_options(&source, "finally_loop_exit_locations.py", &options)
                        .unwrap(),
                    compile_bytes_with_options(
                        source.as_bytes(),
                        "finally_loop_exit_locations.py",
                        &options,
                    )
                    .unwrap(),
                ] {
                    let mut child = Command::new(&python)
                        .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle])
                        .args([
                            target.tag.trim_start_matches('v'),
                            &target.magic_hex(),
                            &source,
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
                        .write_all(&compiled.marshal())
                        .unwrap();
                    let result = child.wait_with_output().unwrap();
                    assert!(
                        result.status.success(),
                        "{source}\n{}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                }
            }
        }
    }
}
