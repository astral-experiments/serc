#[allow(dead_code)]
mod common;

use std::fmt::Write as _;
use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_with_options};

fn assert_matches_cpython(source: &str) {
    for (version, variable, fallback) in [
        (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
        (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
    ] {
        let target = version.target();
        let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let filename = "normal_exit_borrow_boundaries.py";
        let oracle = "import importlib.util, marshal, sys; assert sys.implementation.name == 'cpython'; assert sys.version.split()[0] == sys.argv[1]; assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]; code = compile(sys.stdin.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";
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
        let actual = compile_with_options(source, filename, &options)
            .unwrap()
            .marshal();
        assert!(actual == expected.stdout, "{version:?}:\n{source}");
    }
}

#[test]
fn cold_handler_guard_copies_stop_normal_finally_borrowing() {
    for asynchronous in [false, true] {
        for binding in ["", " as caught"] {
            for leading in ["", "value = obj.ready\n", "work(obj)\n"] {
                for tail in [
                    "if not obj.flag:\n    ready(obj)\n",
                    "if not obj.flag:\n    if obj.other:\n        ready(obj)\n    else:\n        recover(obj)\n",
                    "if not obj.flag:\n    if obj.other:\n        value = obj.ready\n    else:\n        value = obj.other\n",
                ] {
                    for suffix in ["if obj.flag:\n    raise\n", "work(obj)\n"] {
                        let prefix = if asynchronous { "async " } else { "" };
                        let awaited = if asynchronous { "await " } else { "" };
                        let handler = format!(
                            "while obj.ready:\n{}{suffix}",
                            common::indent(&format!("{leading}{tail}")),
                        );
                        let body = format!(
                            "try:\n    while obj.ready:\n        try:\n            {awaited}work(obj)\n        except Other:\n            pass\nexcept Error{binding}:\n{}finally:\n    obj.done = None\n    obj.paused = False\n",
                            common::indent(&handler),
                        );
                        assert_matches_cpython(&format!(
                            "{prefix}def f(obj):\n{}",
                            common::indent(&body),
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn awaited_finalizers_keep_noncopyable_original_cleanup_barriers() {
    for body in ["await work(value)\n", "result = await work(value)\n"] {
        for finalbody in [
            "finish(value)\n",
            "await finish(value)\n",
            "await queue.done(value)\n",
            "await queue.done(value)\nfinish(value)\n",
            "if flag:\n    await queue.done(value)\n",
        ] {
            let body = format!(
                "try:\n{}except FirstError:\n    raise\nexcept Error:\n    recover(value)\n    result = None\nelse:\n    succeeded(value)\nfinally:\n{}return result\n",
                common::indent(body),
                common::indent(finalbody),
            );
            for body in [
                body.clone(),
                format!("with manager:\n{}", common::indent(&body)),
            ] {
                assert_matches_cpython(&format!(
                    "async def f(queue, value, flag, manager):\n{}",
                    common::indent(&body),
                ));
            }
        }
    }
}

#[test]
fn named_continue_cleanup_precedes_a_populated_comprehension_call() {
    for call in [
        "use({item: item for item in values})",
        "value.update({item.name: item for item in other.values})",
        "use([item for item in values])",
        "value.update([item.name for item in values])",
        "use(value, [item for item in values])",
    ] {
        for prefix in ["", "if flag:\n    raise\nrecover(value)\n"] {
            for tail in [
                "return value\n",
                "for element in values:\n    use(element)\n",
            ] {
                let recovery = format!("{prefix}continue\n");
                let handler = format!(
                    "for value in values:\n    try:\n        work(value)\n    except OtherError as nested:\n{}    {call}\n",
                    common::indent(&common::indent(&recovery)),
                );
                let body = format!(
                    "try:\n    work(values)\nexcept Error as error:\n{}else:\n    succeeded(value)\n{tail}",
                    common::indent(&handler),
                );
                for body in [
                    body.clone(),
                    format!("with manager:\n{}", common::indent(&body)),
                    format!(
                        "try:\n{}finally:\n    finish(value)\n",
                        common::indent(&body)
                    ),
                ] {
                    assert_matches_cpython(&format!(
                        "def f(value, other, values, flag, manager):\n{}",
                        common::indent(&body),
                    ));
                }
            }
        }
    }
}

#[test]
fn guarded_named_continue_recovery_keeps_its_unreachable_cleanup() {
    for condition in [
        "flag",
        "value != other",
        "check(value) != other",
        "flag and other",
        "flag or other",
        "not (flag and other)",
    ] {
        for tail in [
            "return value\n",
            "return combine(value)\n",
            "for element in values:\n    use(element)\n",
        ] {
            let handler = format!(
                "for value in values:\n    try:\n        work(value)\n    except OtherError as nested:\n        if {condition}:\n            raise\n        recover(value)\n        continue\n    use(value)\n",
            );
            let body = format!(
                "try:\n    work(values)\nexcept Error as error:\n{}else:\n    succeeded(value)\n{tail}",
                common::indent(&handler),
            );
            for body in [
                body.clone(),
                format!("with manager:\n{}", common::indent(&body)),
                format!(
                    "try:\n{}finally:\n    finish(value)\n",
                    common::indent(&body)
                ),
            ] {
                assert_matches_cpython(&format!(
                    "def f(value, other, values, flag, manager):\n{}",
                    common::indent(&body),
                ));
            }
        }
    }
}

#[test]
fn named_continue_recovery_keeps_its_unreachable_normal_cleanup() {
    for prefix in ["", "recover(value)\n"] {
        for tail in [
            "return None\n",
            "return value\n",
            "return combine(value)\n",
            "return value + other\n",
            "use(value)\nreturn value\n",
            "for element in values:\n    use(element)\n",
        ] {
            let recovery = format!("{prefix}continue\n");
            let handler = format!(
                "for value in values:\n    try:\n        work(value)\n    except OtherError as nested:\n{}    use(value)\n",
                common::indent(&common::indent(&recovery)),
            );
            let body = format!(
                "try:\n    work(values)\nexcept Error as error:\n{}else:\n    succeeded(value)\n{tail}",
                common::indent(&handler),
            );
            for body in [
                body.clone(),
                format!("with manager:\n{}", common::indent(&body)),
                format!(
                    "try:\n{}finally:\n    finish(value)\n",
                    common::indent(&body),
                ),
            ] {
                assert_matches_cpython(&format!(
                    "def f(value, other, values, flag, manager):\n{}",
                    common::indent(&body),
                ));
            }
        }
    }
    for name in ["", " as error"] {
        for else_suite in ["", "else:\n    pass\n"] {
            for suffix in ["use(value)\n", "for other in values:\n    use(other)\n"] {
                let handler = format!(
                    "while True:\n    if flag:\n        break\n    try:\n        work(value)\n    except OtherError as nested:\n        continue\n{}",
                    common::indent(suffix),
                );
                let body = format!(
                    "try:\n    work(values)\nexcept Error{name}:\n{}{else_suite}for element in values:\n    use(element)\n",
                    common::indent(&handler),
                );
                assert_matches_cpython(&format!(
                    "def f(value, values, flag):\n{}",
                    common::indent(&body),
                ));
            }
        }
    }
}

#[test]
fn live_else_suffixes_share_only_the_original_try_range_end() {
    for body in ["await work(value)\n", "value = await work(value)\n"] {
        for exit in ["return", "raise"] {
            for recovery in ["pass\n", "recover(value)\n"] {
                for suffix in [
                    "succeeded(value)\n",
                    "value = value + other\nsucceeded(value)\n",
                    "succeeded(value, other)\n",
                ] {
                    for finalbody in ["value.total += 1\n", "finish(value)\nfinish(other)\n"] {
                        let protected = format!(
                            "try:\n{}except Error as error:\n    if flag:\n        {exit}\nexcept OtherError as error:\n{}else:\n{}finally:\n{}",
                            common::indent(body),
                            common::indent(recovery),
                            common::indent(suffix),
                            common::indent(finalbody),
                        );
                        assert_matches_cpython(&format!(
                            "async def f(value, other, flag):\n{}",
                            common::indent(&protected),
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn runtime_while_false_exits_share_only_authored_break_aliases() {
    for (condition, loop_body) in [
        ("flag", "if other:\n    break\n"),
        ("check(value)", "if other:\n    break\n"),
        ("True", "if other:\n    break\n"),
        ("flag", "if other:\n    break\nwork(value)\n"),
        ("check(value)", "if other:\n    break\nwork(value)\n"),
        ("flag and other", "if other:\n    break\nwork(value)\n"),
        ("True", "if other:\n    break\nwork(value)\n"),
    ] {
        for named in [false, true] {
            for cleanup in [
                "finish(value)\n",
                "if value is not None:\n    finish(value)\nwork(value)\n",
                "if value is not None:\n    pass\nfinish(value)\n",
            ] {
                let name = if named { " as error" } else { "" };
                let protected = format!(
                    "try:\n    while {condition}:\n{}except Error{name}:\n    pass\nfinally:\n{}",
                    common::indent(&common::indent(loop_body)),
                    common::indent(cleanup),
                );
                for body in [
                    protected.clone(),
                    format!("with manager:\n{}", common::indent(&protected)),
                ] {
                    assert_matches_cpython(&format!(
                        "def f(value, flag, other, manager):\n{}",
                        common::indent(&body),
                    ));
                }
            }
        }
    }
}

#[test]
fn runtime_while_conditions_enter_after_emptied_recovery_backedges() {
    for condition in ["flag", "check(value)", "flag and other", "True"] {
        for named in [false, true] {
            for recovery in ["pass", "recover(value)"] {
                for cleanup in [
                    "finish(value)\n",
                    "if value is not None:\n    finish(value)\nwork(value)\n",
                    "if value is not None:\n    pass\nfinish(value)\n",
                ] {
                    let name = if named { " as error" } else { "" };
                    let protected = format!(
                        "try:\n    while {condition}:\n        try:\n            pass\n        except InnerError:\n            pass\nexcept Error{name}:\n    {recovery}\nfinally:\n{}",
                        common::indent(cleanup)
                    );
                    for body in [
                        protected.clone(),
                        format!("with manager:\n{}", common::indent(&protected)),
                    ] {
                        assert_matches_cpython(&format!(
                            "def f(value, flag, other, manager):\n{}",
                            common::indent(&body)
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn conditional_value_joins_keep_original_finalizer_sources() {
    for condition in ["flag", "flag and other", "True"] {
        for consumer in ["store", "local_call", "global_call", "call_chain"] {
            let value = format!("left if {condition} else right");
            let prefix = match consumer {
                "store" => format!("result = {value}\n"),
                "local_call" => format!("local({value})\n"),
                "global_call" => format!("work({value})\n"),
                _ => format!("work({value}).done()\n"),
            };
            for context in ["plain", "with", "with_while"] {
                for cleanup in ["finish(value)\n", "return value\n"] {
                    for named in [false, true] {
                        let name = if named { " as error" } else { "" };
                        let mut body = format!(
                            "try:\n{}except Error{name}:\n    if other:\n        raise Other(value) from None\nfinally:\n{}",
                            common::indent(&format!("{prefix}work(value)\n")),
                            common::indent(cleanup),
                        );
                        if context == "with_while" {
                            body = format!("while repeat:\n{}", common::indent(&body));
                        }
                        if context != "plain" {
                            body = format!("with manager:\n{}", common::indent(&body));
                        }
                        assert_matches_cpython(&format!(
                            "def f(value, left, right, flag, other, repeat, local, manager):\n{}",
                            common::indent(&body),
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn fused_local_prefixes_keep_their_original_finalizer_source() {
    for tail in [
        "first, second = work(value)\n",
        "first = work(value, flag)\n",
        "first = value; second = first\n",
        "first = value\nsecond = first\n",
    ] {
        for large_slots in [false, true] {
            let mut padding = String::new();
            if large_slots {
                for index in 0..16 {
                    write!(padding, "padding{index}, ").unwrap();
                }
            }
            for handler in ["pass_named", "raise_named", "raise_unnamed", "return_named"] {
                for cleanup in [
                    "finish(value)\n",
                    "if value is not None and not value.done():\n    finish(value)\n",
                ] {
                    for scope in ["sync", "async", "generator"] {
                        for in_loop in [false, true] {
                            let name = if handler == "raise_unnamed" {
                                ""
                            } else {
                                " as error"
                            };
                            let recovery = match handler {
                                "pass_named" => "pass\n",
                                "return_named" => "if flag:\n    return value\n",
                                _ => "if flag:\n    raise Other(value) from None\n",
                            };
                            let mut body = format!(
                                "try:\n    if flag:\n        pass\n    else:\n        pass\n{}except Error{name}:\n{}finally:\n{}",
                                common::indent(tail),
                                common::indent(recovery),
                                common::indent(cleanup),
                            );
                            if in_loop {
                                body = format!("while repeat:\n{}", common::indent(&body));
                            }
                            if scope == "generator" {
                                body = format!("yield value\n{body}");
                            }
                            let prefix = if scope == "async" { "async " } else { "" };
                            assert_matches_cpython(&format!(
                                "{prefix}def f({padding}value, flag, repeat):\n{}",
                                common::indent(&body),
                            ));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn checked_loads_keep_retained_finalizer_source_and_target_identity() {
    for binding in ["parameter", "unset", "initialized", "deleted"] {
        for tail in [
            "response = work(value)\n",
            "response = pending.result()\n",
            "response = await work(value)\n",
            "response = await pending.result()\n",
        ] {
            for handler in ["raise_named", "raise_unnamed", "return_named"] {
                for condition in [
                    "pending is not None",
                    "pending is not None and not pending.done()",
                ] {
                    for in_loop in [false, true] {
                        let name = if handler == "raise_unnamed" {
                            ""
                        } else {
                            " as error"
                        };
                        let exit = if handler == "return_named" {
                            "return value"
                        } else {
                            "raise Other(code) from None"
                        };
                        let mut body = format!(
                            "try:\n    if flag:\n        pass\n    else:\n        pending = work(value)\n{}except Error{name}:\n    if code in (1, 2):\n        {exit}\nfinally:\n    if {condition}:\n        finish(pending)\n",
                            common::indent(tail)
                        );
                        if in_loop {
                            body = format!("while repeat:\n{}", common::indent(&body));
                        }
                        let initialize = match binding {
                            "initialized" => "pending = None\n",
                            "deleted" => "pending = None\ndel pending\n",
                            _ => "",
                        };
                        let parameter = if binding == "parameter" {
                            ", pending"
                        } else {
                            ""
                        };
                        assert_matches_cpython(&format!(
                            "async def f(flag, repeat, value, code{parameter}):\n{}",
                            common::indent(&format!("{initialize}{body}"))
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn checked_local_conditions_keep_their_original_finalizer_entry() {
    for binding in ["parameter", "unset", "initialized", "deleted"] {
        for protected in ["pass", "observe(value)"] {
            for condition in [
                "pending is not None",
                "pending",
                "pending.value",
                "pending is not None and not pending.done()",
            ] {
                for cleanup in ["pass", "finish(pending)"] {
                    for scope in ["sync", "async", "generator"] {
                        for in_loop in [false, true] {
                            let mut body = format!(
                                "try:\n    if flag:\n        {protected}\n    else:\n        pending = work(value)\nexcept Error as error:\n    pass\nfinally:\n    if {condition}:\n        {cleanup}\n"
                            );
                            if in_loop {
                                body = format!("while repeat:\n{}", common::indent(&body));
                            }
                            let initialize = match binding {
                                "initialized" => "pending = None\n",
                                "deleted" => "pending = None\ndel pending\n",
                                _ => "",
                            };
                            body = format!("{initialize}{body}");
                            if scope == "generator" {
                                body = format!("yield value\n{body}");
                            }
                            let prefix = if scope == "async" { "async " } else { "" };
                            let parameter = if binding == "parameter" {
                                ", pending"
                            } else {
                                ""
                            };
                            assert_matches_cpython(&format!(
                                "{prefix}def f(flag, repeat, value{parameter}):\n{}",
                                common::indent(&body)
                            ));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn unnamed_raising_guards_retain_the_nonempty_finalizer_entry() {
    for scope in ["sync", "async"] {
        for manager in ["plain", "sync", "async"] {
            if manager == "async" && scope != "async" {
                continue;
            }
            for named in [false, true] {
                for protected in ["pass\n", "work(value)\n"] {
                    for condition in ["flag", "receiver.read() != value"] {
                        for cleanup in ["cleanup(value)\n", "receiver.finish(enabled=True)\n"] {
                            let name = if named { " as error" } else { "" };
                            let mut body = format!(
                                "try:\n{}except Error{name}:\n    if {condition}:\n        raise Other(value) from None\nfinally:\n{}finish(value)\n",
                                common::indent(protected),
                                common::indent(cleanup)
                            );
                            if manager != "plain" {
                                let prefix = if manager == "async" { "async " } else { "" };
                                body = format!(
                                    "{prefix}with manager() as receiver:\n{}",
                                    common::indent(&body)
                                );
                            }
                            let prefix = if scope == "async" { "async " } else { "" };
                            assert_matches_cpython(&format!(
                                "{prefix}def f(flag, value, receiver):\n{}",
                                common::indent(&body)
                            ));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn specialized_super_calls_keep_their_original_finalizer_entry() {
    for scope in ["sync", "async", "generator"] {
        for captured in [false, true] {
            for protected in [
                "if flag:\n    pass\nelse:\n    pass\n",
                "if flag:\n    if other:\n        pass\nelse:\n    pass\n",
            ] {
                for cleanup in [
                    "super().finish()\n",
                    "super().finish(value)\n",
                    "super(Probe, self).finish()\n",
                ] {
                    let mut body = format!(
                        "try:\n{}except Error:\n    pass\nfinally:\n{}",
                        common::indent(protected),
                        common::indent(cleanup)
                    );
                    if captured {
                        body = format!("def capture():\n    return self\n{body}");
                    }
                    if scope == "generator" {
                        body = format!("yield value\n{body}");
                    }
                    let prefix = if scope == "async" { "async " } else { "" };
                    let method = format!(
                        "{prefix}def f(self, flag, other, value):\n{}",
                        common::indent(&body)
                    );
                    assert_matches_cpython(&format!("class Probe:\n{}", common::indent(&method)));
                }
            }
        }
    }
}

#[test]
fn discarded_call_returns_keep_their_original_finalizer_entry() {
    for scope in ["sync", "async", "generator"] {
        for named in [false, true] {
            for captured in [false, true] {
                for protected in [
                    "if flag:\n    pass\nelse:\n    pass\n",
                    "if flag:\n    if other:\n        pass\nelse:\n    pass\n",
                ] {
                    for cleanup in [
                        "finish(value)\n",
                        "cleanup(value)\n",
                        "target.finish(value)\n",
                        "target.finish()\n",
                    ] {
                        let name = if named { " as error" } else { "" };
                        let mut body = format!(
                            "try:\n{}except Error{name}:\n    pass\nfinally:\n{}",
                            common::indent(protected),
                            common::indent(cleanup)
                        );
                        if captured {
                            body = format!("def capture():\n    return target\n{body}");
                        }
                        if scope == "generator" {
                            body = format!("yield None\n{body}");
                        }
                        let prefix = if scope == "async" { "async " } else { "" };
                        assert_matches_cpython(&format!(
                            "{prefix}def f(flag, other, target, value, cleanup):\n{}",
                            common::indent(&body)
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn unfolded_global_call_prefixes_exceed_the_copy_budget() {
    for scope in ["sync", "async", "generator"] {
        for named in [false, true] {
            for handler in [
                "try:\n    recover(value)\nfinally:\n    pass\n",
                "if flag:\n    pass\nelse:\n    with manager():\n        recover(value)\n",
            ] {
                for cleanup in [
                    "finish(value)\n",
                    "finish(value, flag)\n",
                    "cleanup(value)\n",
                    "service.finish(value)\n",
                ] {
                    for wrapper in ["plain", "for", "finally", "handler"] {
                        let name = if named { " as error" } else { "" };
                        let recovery = format!(
                            "try:\n    work(value)\nexcept Error{name}:\n{}",
                            common::indent(handler)
                        );
                        let body = format!(
                            "try:\n{}finally:\n{}",
                            common::indent(&recovery),
                            common::indent(cleanup)
                        );
                        let mut body = match wrapper {
                            "for" => format!("for item in items:\n{}", common::indent(&body)),
                            "finally" => format!(
                                "try:\n{}finally:\n    outer(value)\n",
                                common::indent(&body)
                            ),
                            "handler" => format!(
                                "try:\n{}except Outer as caught:\n    pass\n",
                                common::indent(&body)
                            ),
                            _ => body,
                        };
                        if scope == "generator" {
                            body = format!("yield None\n{body}");
                        }
                        let prefix = if scope == "async" { "async " } else { "" };
                        assert_matches_cpython(&format!(
                            "{prefix}def f(flag, value, items, cleanup):\n{}",
                            common::indent(&body)
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn terminal_nested_recovery_keeps_the_enclosing_finalizer_successor() {
    for scope in ["sync", "async", "generator"] {
        for named in [false, true] {
            for protected in ["work(value)\n", "if flag:\n    work(value)\n"] {
                for cleanup in [
                    "finish(value)\n",
                    "if value is not None:\n    finish(value)\n",
                ] {
                    for wrapper in ["plain", "for", "manager", "finally", "handler"] {
                        let name = if named { " as error" } else { "" };
                        let recovery = format!(
                            "try:\n{}except Error{name}:\n    try:\n        recover(value)\n    except Other:\n        pass\n",
                            common::indent(protected)
                        );
                        let body = format!(
                            "try:\n{}finally:\n{}",
                            common::indent(&recovery),
                            common::indent(cleanup)
                        );
                        let mut body = match wrapper {
                            "for" => format!("for item in items:\n{}", common::indent(&body)),
                            "manager" => {
                                format!("with manager():\n{}", common::indent(&body))
                            }
                            "finally" => format!(
                                "try:\n{}finally:\n    outer(value)\n",
                                common::indent(&body)
                            ),
                            "handler" => format!(
                                "try:\n{}except LastError:\n    pass\n",
                                common::indent(&body)
                            ),
                            _ => body,
                        };
                        if scope == "generator" {
                            body = format!("yield value\n{body}");
                        }
                        let prefix = if scope == "async" { "async " } else { "" };
                        assert_matches_cpython(&format!(
                            "{prefix}def f(flag, value, items):\n{}",
                            common::indent(&body)
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn terminal_else_manager_preserves_the_empty_handler_cleanup() {
    for scope in ["sync", "async", "generator"] {
        for named in [false, true] {
            for earlier_arm in [
                "pass\n",
                "observe(value)\n",
                "return value\n",
                "if flag:\n    observe(value)\n",
            ] {
                for has_else in [false, true] {
                    for has_suffix in [false, true] {
                        for cleanup in [
                            "finish(value)\n",
                            "if value is not None:\n    finish(value)\n",
                        ] {
                            let name = if named { " as error" } else { "" };
                            let mut handler = format!(
                                "if other:\n{}else:\n    with manager():\n        recover(value)\n",
                                common::indent(earlier_arm)
                            );
                            if has_suffix {
                                handler.push_str("observe(value)\n");
                            }
                            let mut recovery = format!(
                                "try:\n    work(value)\nexcept Error{name}:\n{}",
                                common::indent(&handler)
                            );
                            if has_else {
                                recovery.push_str("else:\n    pass\n");
                            }
                            let mut body = format!(
                                "try:\n{}finally:\n{}",
                                common::indent(&recovery),
                                common::indent(cleanup)
                            );
                            if scope == "generator" {
                                body = format!("yield value\n{body}");
                            }
                            let prefix = if scope == "async" { "async " } else { "" };
                            assert_matches_cpython(&format!(
                                "{prefix}def f(flag, other, value):\n{}",
                                common::indent(&body)
                            ));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn terminal_handler_finalizer_leaves_the_original_cleanup_empty() {
    for scope in ["sync", "async", "generator"] {
        for named in [false, true] {
            for finalizer in [
                "pass\n",
                "observe(value)\n",
                "if flag:\n    observe(value)\n",
            ] {
                for cleanup in [
                    "finish(value)\n",
                    "if value is not None:\n    finish(value)\n",
                ] {
                    let name = if named { " as error" } else { "" };
                    let handler = format!(
                        "try:\n    recover(value)\nfinally:\n{}",
                        common::indent(finalizer)
                    );
                    let recovery = format!(
                        "try:\n    work(value)\nexcept Error{name}:\n{}",
                        common::indent(&handler)
                    );
                    let mut body = format!(
                        "try:\n{}finally:\n{}",
                        common::indent(&recovery),
                        common::indent(cleanup)
                    );
                    if scope == "generator" {
                        body = format!("yield value\n{body}");
                    }
                    let prefix = if scope == "async" { "async " } else { "" };
                    assert_matches_cpython(&format!(
                        "{prefix}def f(flag, value):\n{}",
                        common::indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn copied_short_finalizers_do_not_inherit_recovery_borrow_barriers() {
    for scope in ["sync", "async", "generator"] {
        for named in [false, true] {
            for cleanup in ["pass\n", "return value\n", "return None\n"] {
                for indirect in [false, true] {
                    for suffix in [false, true] {
                        let name = if named { " as error" } else { "" };
                        let mut body = format!(
                            "try:\n    work(value)\nexcept Error{name}:\n    try:\n        recover(value)\n    except Other:\n        pass\n"
                        );
                        if indirect {
                            body = format!("if flag:\n{}", common::indent(&body));
                        }
                        if suffix {
                            body.push_str("observe(value)\n");
                        }
                        body = format!(
                            "try:\n{}finally:\n{}",
                            common::indent(&body),
                            common::indent(cleanup)
                        );
                        if scope == "generator" {
                            body = format!("yield value\n{body}");
                        }
                        let prefix = if scope == "async" { "async " } else { "" };
                        assert_matches_cpython(&format!(
                            "{prefix}def f(flag, value):\n{}",
                            common::indent(&body)
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn terminating_recovery_else_keeps_the_original_empty_borrow_array() {
    for terminal in ["raise Error(value)", "return value"] {
        for suffix in ["result = value", "target.recover(value)"] {
            for named in [false, true] {
                for wrapper in ["", "with manager():", "for item in items:"] {
                    for successor in [
                        "target.finish(value)\n",
                        "if flag and value:\n    target.finish(value)\n",
                    ] {
                        let outer_name = if named { " as error" } else { "" };
                        let inner_name = if named { " as nested_error" } else { "" };
                        let mut body = format!(
                            "try:\n    target.work(value)\nexcept Error{outer_name}:\n    try:\n        target.work(value)\n    except Other{inner_name}:\n        pass\n    else:\n        {terminal}\n    {suffix}\n{successor}"
                        );
                        if !wrapper.is_empty() {
                            body = format!("{wrapper}\n{}", common::indent(&body));
                        }
                        assert_matches_cpython(&format!(
                            "def f(target, value, flag, items):\n{}",
                            common::indent(&body)
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn named_handler_return_keeps_copied_finalizer_condition_cells() {
    for scope in ["def", "async def"] {
        for condition in ["flag", "flag or value", "flag is not None"] {
            for protected in ["pass", "work()"] {
                for another_handler in [false, true] {
                    let other = if another_handler {
                        "    except LastError:\n        pass\n"
                    } else {
                        ""
                    };
                    let body = format!(
                        "with manager():\n    try:\n        while True:\n            try:\n                {protected}\n            except Error as error:\n                if {condition}:\n                    return\n    except OtherError:\n        pass\n{other}    finally:\n        if not flag:\n            finish()\n"
                    );
                    assert_matches_cpython(&format!(
                        "{scope} f(flag, value):\n{}",
                        common::indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn raising_handler_guards_keep_populated_normal_finalizers_reachable() {
    for finalizer in [
        "target.use(value)\n",
        "target.value = value\n",
        "with manager():\n    target.use(value)\n",
        "try:\n    target.use(value)\nexcept Error:\n    pass\n",
        "if target is not None:\n    target.use(value)\n",
        "for item in items:\n    target.use(item)\n",
        "while flag:\n    target.use(value)\n",
    ] {
        for raised in ["raise", "raise error", "raise Error(value) from error"] {
            for outer in ["", "with manager():", "async with manager():"] {
                for body in ["pass", "work(value)"] {
                    let mut body = format!(
                        "try:\n    {body}\nexcept Error as error:\n    if flag:\n        {raised}\nfinally:\n{}",
                        common::indent(finalizer)
                    );
                    if !outer.is_empty() {
                        body = format!("{outer}\n{}", common::indent(&body));
                    }
                    let prefix = if outer.starts_with("async") {
                        "async "
                    } else {
                        ""
                    };
                    assert_matches_cpython(&format!(
                        "{prefix}def f(flag, value, target, items):\n{}",
                        common::indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn asserted_else_success_keeps_its_retained_normal_exit_reachable() {
    for test in [
        "value",
        "value is None",
        "value is not None",
        "value is other",
        "value == other",
        "a and b",
        "low < value < high",
        "False",
    ] {
        for handler in [
            "pass\n",
            "assert error is value\n",
            "target.recover(value)\n",
            "raise\n",
            "if flag:\n    raise\n",
        ] {
            for named in [false, true] {
                for suspended in [false, true] {
                    let name = if named { " as error" } else { "" };
                    let prefix = if suspended { "async " } else { "" };
                    let awaited = if suspended { "await " } else { "" };
                    let body = format!(
                        "try:\n    {awaited}work(value)\nexcept Error{name}:\n{}else:\n    assert {test}\ntarget.use(value)\n",
                        common::indent(handler)
                    );
                    assert_matches_cpython(&format!(
                        "{prefix}def f(value, target, other, a, b, low, high, flag):\n{}",
                        common::indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn manager_cleanup_absorbs_a_single_false_edge_loop_exit() {
    for test in ["flag", "a or b", "value < other"] {
        for handler in [
            "pass\n",
            "raise\n",
            "target.recover(value)\nraise\n",
            "for item in items:\n    target.recover(item)\nraise\n",
        ] {
            for depth in 0..=3 {
                for work in ["pass", "target.use(value)"] {
                    let mut body = format!(
                        "try:\n    while {test}:\n        {work}\nexcept Error:\n{}",
                        common::indent(handler)
                    );
                    if depth > 0 {
                        let managers = vec!["manager()"; depth].join(", ");
                        body = format!("with {managers}:\n{}", common::indent(&body));
                    }
                    body.push_str("target.finish(value)\n");
                    assert_matches_cpython(&format!(
                        "def f(flag, a, b, value, other, target, items):\n{}",
                        common::indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn terminal_handlers_leave_the_manager_pop_as_the_only_loop_exit() {
    for handler in [
        "if flag:\n    pass\nraise\n",
        "if flag:\n    target.recover(value)\nraise\n",
        "if flag:\n    target.recover(value)\nelse:\n    target.other(value)\nraise\n",
        "result = tuple(item for item in items if item)\nraise\n",
        "result = value if flag else other\nraise\n",
        "try:\n    target.recover(value)\nexcept Other:\n    pass\nraise\n",
        "try:\n    target.recover(value)\nfinally:\n    target.other(value)\nraise\n",
        "with manager():\n    target.recover(value)\nraise\n",
        "if flag:\n    return value\nraise\n",
        "return value\n",
        "return target.recover(value)\n",
        "return value if flag else other\n",
    ] {
        for test in ["flag", "a or b"] {
            for depth in 0..=2 {
                for named in [false, true] {
                    let name = if named { " as error" } else { "" };
                    let mut body = format!(
                        "try:\n    while {test}:\n        target.use(value)\nexcept Error{name}:\n{}",
                        common::indent(handler)
                    );
                    for _ in 0..depth {
                        body = format!("with manager():\n{}", common::indent(&body));
                    }
                    body.push_str("target.finish(value)\n");
                    assert_matches_cpython(&format!(
                        "def f(flag, a, b, value, other, target, items):\n{}",
                        common::indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn nested_conditional_body_preserves_the_empty_finalizer_entry() {
    for test in [
        "flag",
        "flag and other",
        "flag or other",
        "not flag",
        "value is None",
        "value is not None",
        "low < value < high",
        "flag and not other",
    ] {
        for work in ["pass", "target.work(value)"] {
            for handler in [
                "pass\n",
                "target.recover(value)\n",
                "raise\n",
                "if other:\n    raise\ntarget.recover(value)\n",
            ] {
                for named in [false, true] {
                    for cleanup in [
                        "target.finish(value)\n",
                        "if target is not None:\n    target.finish(value)\n",
                    ] {
                        let name = if named { " as error" } else { "" };
                        let inner = format!(
                            "try:\n    if {test}:\n        {work}\nexcept Error{name}:\n{}",
                            common::indent(handler)
                        );
                        let body = format!(
                            "try:\n{}finally:\n{}",
                            common::indent(&inner),
                            common::indent(cleanup)
                        );
                        assert_matches_cpython(&format!(
                            "def f(target, value, flag, other, low, high):\n{}",
                            common::indent(&body)
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn completed_prefixes_do_not_reopen_a_nested_conditional_exit() {
    for prefix in [
        "if other:\n    result = value\n",
        "if other:\n    raise Error(value)\n",
        "if other:\n    return value\n",
        "for item in items:\n    target.work(item)\n",
    ] {
        for test in ["flag", "flag and other", "low < value < high"] {
            for inside in [false, true] {
                let protected = format!("if {test}:\n    target.work(value)\n");
                let protected = if inside {
                    format!("{prefix}{protected}")
                } else {
                    protected
                };
                let inner = format!(
                    "try:\n{}except Error as error:\n    if other:\n        raise\n    if flag:\n        raise\n    result = value\n",
                    common::indent(&protected)
                );
                let inner = if inside {
                    inner
                } else {
                    format!("{prefix}{inner}")
                };
                let body = format!(
                    "try:\n{}finally:\n    if target is not None:\n        target.finish(value)\n",
                    common::indent(&inner)
                );
                assert_matches_cpython(&format!(
                    "def f(target, value, flag, other, low, high, items):\n{}",
                    common::indent(&body)
                ));
            }
        }
    }
}

#[test]
fn enclosing_manager_and_loop_depths_preserve_live_break_cleanup() {
    for depth in 0..=3 {
        for outer_loop in ["", "for outer_item in items:", "while outer_flag:"] {
            for (declaration, manager, prefix) in [
                ("def", "with", ""),
                ("async def", "with", ""),
                ("async def", "async with", ""),
                ("def", "with", "yield value\n"),
            ] {
                for cleanup in [
                    "target.finish(value)\n",
                    "if target is not None:\n    target.finish(value)\n",
                    "for item in items:\n    target.finish(item)\n",
                    "result = target.finish(value if other else None)\n",
                ] {
                    let mut body = format!(
                        "try:\n    while True:\n        if not flag:\n            result = target.choose(value)\n            break\n        target.work(value)\nfinally:\n{}",
                        common::indent(cleanup)
                    );
                    if !outer_loop.is_empty() {
                        body = format!("{outer_loop}\n{}", common::indent(&body));
                    }
                    for _ in 0..depth {
                        body = format!("{manager} manager():\n{}", common::indent(&body));
                    }
                    assert_matches_cpython(&format!(
                        "{declaration} f(target, value, flag, other, outer_flag, items):\n{}",
                        common::indent(&format!("{prefix}{body}"))
                    ));
                }
            }
        }
    }
}

#[test]
fn manager_depth_does_not_reopen_removed_break_or_owned_cleanup() {
    for depth in 1..=3 {
        for outer_loop in ["", "for outer_item in items:", "while outer_flag:"] {
            for while_body in ["break\n", "if flag:\n    break\n"] {
                for (cleanup, owned) in [
                    ("if other:\n    return\n", false),
                    ("if other:\n    raise Error(value)\n", false),
                    ("if False:\n    target.finish(value)\n", false),
                    ("while other:\n    target.finish(value)\n", false),
                    ("target.finish(value)\nreturn\n", true),
                    ("target.finish(value)\nraise Error(value)\n", true),
                    ("while other:\n    target.finish(value)\n", true),
                ] {
                    let mut body = format!(
                        "try:\n    while True:\n{}finally:\n{}",
                        common::indent(&common::indent(while_body)),
                        common::indent(cleanup)
                    );
                    if !outer_loop.is_empty() {
                        body = format!("{outer_loop}\n{}", common::indent(&body));
                    }
                    for _ in 0..depth {
                        body = format!("with manager():\n{}", common::indent(&body));
                    }
                    if owned {
                        body = format!(
                            "try:\n    target.begin(value)\nexcept Outer:\n{}",
                            common::indent(&body)
                        );
                    }
                    assert_matches_cpython(&format!(
                        "def f(target, value, flag, other, outer_flag, items):\n{}",
                        common::indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn live_loop_break_enters_a_populated_manager_finalizer() {
    for (prefix, test) in [
        ("", "True"),
        ("async ", "True"),
        ("async ", "flag"),
        ("async ", "a and b"),
    ] {
        for handler in [
            "pass\n",
            "target.recover(value)\n",
            "raise\n",
            "if flag:\n    raise\n",
        ] {
            let body = format!(
                "try:\n    while {test}:\n        if a or b:\n            break\n        work(value)\nexcept Error as error:\n{}finally:\n    with manager():\n        target.use(value)\n",
                common::indent(handler)
            );
            assert_matches_cpython(&format!(
                "{prefix}def f(value, target, a, b, flag):\n{}",
                common::indent(&body)
            ));
        }
    }
}

#[test]
fn retained_manager_pop_does_not_repopulate_a_finalizer_loop_exit() {
    for cleanup in [
        "for item in items:\n    with manager():\n        target.use(item)\n",
        "for item in items:\n    target.use(item)\n",
        "for item in items:\n    try:\n        target.use(item)\n    except Error:\n        pass\n",
    ] {
        for handler in [
            "",
            "except Error:\n    pass\n",
            "except Error:\n    target.recover(value)\n",
        ] {
            for depth in 0..=3 {
                for protected in ["pass", "target.use(value)"] {
                    let mut body = format!(
                        "try:\n    {protected}\n{handler}finally:\n{}",
                        common::indent(cleanup)
                    );
                    if depth > 0 {
                        let managers = vec!["manager()"; depth].join(", ");
                        body = format!("with {managers}:\n{}", common::indent(&body));
                    }
                    body.push_str("target.finish(value)\n");
                    assert_matches_cpython(&format!(
                        "def f(value, target, items):\n{}",
                        common::indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn completed_handler_managers_leave_populated_finalizer_routes() {
    for cleanup in [
        "target.finish(value)\n",
        "if flag:\n    target.finish(value)\n",
        "with manager():\n    target.finish(value)\n",
        "for item in items:\n    target.finish(item)\n",
    ] {
        for manager_body in ["pass\n", "target.use(value)\n"] {
            for suffix in [
                "",
                "target.recover(value)\n",
                "if flag:\n    raise\ntarget.recover(value)\n",
            ] {
                for (prefix, outer) in [
                    ("", ""),
                    ("async ", ""),
                    ("", "with manager():"),
                    ("async ", "with manager():"),
                    ("async ", "async with manager():"),
                ] {
                    let handler = format!(
                        "if flag:\n    raise\nwith manager():\n{}{suffix}",
                        common::indent(manager_body)
                    );
                    let mut body = format!(
                        "try:\n    work(value)\nexcept Error as error:\n{}finally:\n{}",
                        common::indent(&handler),
                        common::indent(cleanup)
                    );
                    if !outer.is_empty() {
                        body = format!("{outer}\n{}", common::indent(&body));
                    }
                    assert_matches_cpython(&format!(
                        "{prefix}def f(flag, value, target, items):\n{}",
                        common::indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn nested_conditional_else_preserves_the_empty_finalizer_entry() {
    for alternative in [
        "if flag:\n    pass\n",
        "if flag:\n    target.use(value)\n",
        "if flag:\n    target.use(value)\nelse:\n    target.use(other)\n",
        "pass\n",
        "target.use(value)\n",
    ] {
        for handler in [
            "pass\n",
            "target.recover(value)\n",
            "raise\n",
            "if flag and other:\n    pass\n",
            "if flag and other:\n    target.recover(value)\n",
            "if flag and other:\n    target.recover(value)\nraise\n",
        ] {
            for (prefix, work) in [
                ("", "work(value)"),
                ("async ", "await work(value)"),
                ("", "yield value"),
                ("async ", "yield value"),
            ] {
                for named in [false, true] {
                    let name = if named { " as error" } else { "" };
                    let inner = format!(
                        "try:\n    {work}\nexcept Error{name}:\n{}else:\n{}",
                        common::indent(handler),
                        common::indent(alternative)
                    );
                    let body = format!(
                        "try:\n{}finally:\n    target.finish(value)\n",
                        common::indent(&inner)
                    );
                    assert_matches_cpython(&format!(
                        "{prefix}def f(value, target, flag, other):\n{}",
                        common::indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn guarded_recovery_alternatives_keep_the_handler_cleanup_live() {
    for branches in [1, 4] {
        for extra_parameters in [0, 16] {
            for named in [false, true] {
                for preceding_handler in [false, true] {
                    for suspended in [false, true] {
                        for raising_guards in [false, true] {
                            let guard = if raising_guards {
                                "if other:\n    raise\n"
                            } else {
                                ""
                            };
                            let mut alternatives = String::new();
                            for index in 0..branches {
                                let keyword = if index == 0 { "if" } else { "elif" };
                                write!(alternatives, "{keyword} value == {index}:\n    pass\n")
                                    .unwrap();
                            }
                            let recovery = format!(
                                "try:\n    pass\nexcept NestedError:\n{}    target.recover(value)\n",
                                common::indent(guard)
                            );
                            write!(alternatives, "else:\n{}", common::indent(&recovery)).unwrap();
                            let handler =
                                format!("{guard}if flag:\n{}", common::indent(&alternatives));
                            let preceding = if preceding_handler {
                                format!(
                                    "except Other:\n{}    target.observe(value)\n",
                                    common::indent(guard)
                                )
                            } else {
                                String::new()
                            };
                            let name = if named { " as error" } else { "" };
                            let body = format!(
                                "try:\n    work(value)\n{preceding}except Error{name}:\n{}target.finish(value)\n",
                                common::indent(&handler)
                            );
                            let prefix = if suspended { "async " } else { "" };
                            let mut parameters = (0..extra_parameters)
                                .map(|index| format!("arg{index}"))
                                .collect::<Vec<_>>();
                            parameters
                                .extend(["value", "target", "flag", "other"].map(str::to_owned));
                            assert_matches_cpython(&format!(
                                "{prefix}def f({}):\n{}",
                                parameters.join(", "),
                                common::indent(&body)
                            ));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn unconditionally_copied_recovery_joins_keep_their_borrow_barrier() {
    let recovery = "try:\n    pass\nexcept NestedError:\n    target.recover(value)\n";
    for named in [false, true] {
        for suspended in [false, true] {
            let final_alternative = format!(
                "if value == 0:\n    pass\nelse:\n{}",
                common::indent(recovery)
            );
            let first_alternative = format!(
                "if value == 0:\n{}else:\n    pass\n",
                common::indent(recovery)
            );
            for handler in [
                recovery.to_owned(),
                format!(
                    "if flag:\n{}else:\n    pass\n",
                    common::indent(&final_alternative)
                ),
                format!("if flag:\n{}", common::indent(&first_alternative)),
            ] {
                let name = if named { " as error" } else { "" };
                let body = format!(
                    "try:\n    work(value)\nexcept Error{name}:\n{}target.finish(value)\n",
                    common::indent(&handler)
                );
                let prefix = if suspended { "async " } else { "" };
                assert_matches_cpython(&format!(
                    "{prefix}def f(value, target, flag):\n{}",
                    common::indent(&body)
                ));
            }
        }
    }
}

#[test]
fn nonfinal_recovery_arms_keep_empty_exits_after_finalizer_work() {
    for condition in ["first", "first and second", "first or second"] {
        for alternate in [
            "else:\n    pass\n",
            "elif second:\n    target.observe(value)\n",
        ] {
            for protected in [
                "pass\n",
                "target.work(value)\n",
                "with manager():\n    target.work(value)\n",
            ] {
                for named in [false, true] {
                    for handler in ["", "except Error as error:\n    target.recover(value)\n"] {
                        for suffix in ["target.finish(value)\n", "result = target.finish(value)\n"]
                        {
                            let name = if named { " as recovered" } else { "" };
                            let recovery = format!(
                                "try:\n{}except Error{name}:\n    pass\n",
                                common::indent(protected)
                            );
                            let cleanup = format!(
                                "if {condition}:\n{}{alternate}{suffix}",
                                common::indent(&recovery)
                            );
                            let body = format!(
                                "try:\n    work(value)\n{handler}finally:\n{}target.use(value)\n",
                                common::indent(&cleanup)
                            );
                            assert_matches_cpython(&format!(
                                "def f(value, target, first, second):\n{}",
                                common::indent(&body)
                            ));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn final_recovery_arms_keep_their_conditional_cleanup_bypass() {
    for condition in ["first", "first and second", "first or second"] {
        for protected in ["pass\n", "target.work(value)\n"] {
            for named in [false, true] {
                for handler in ["", "except Error as error:\n    target.recover(value)\n"] {
                    let name = if named { " as recovered" } else { "" };
                    let recovery = format!(
                        "try:\n{}except Error{name}:\n    pass\n",
                        common::indent(protected)
                    );
                    let cleanup = format!(
                        "if {condition}:\n{}target.finish(value)\n",
                        common::indent(&recovery)
                    );
                    let body = format!(
                        "try:\n    work(value)\n{handler}finally:\n{}target.use(value)\n",
                        common::indent(&cleanup)
                    );
                    assert_matches_cpython(&format!(
                        "def f(value, target, first, second):\n{}",
                        common::indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn guarded_recovery_reaches_populated_managed_finalizers() {
    let recovery = "try:\n    pass\nexcept NestedError:\n    target.recover(value)\n";
    for handler in [
        recovery.to_owned(),
        format!("if flag:\n{}", common::indent(recovery)),
        format!(
            "if flag:\n    if other:\n        pass\n    else:\n{}",
            common::indent(&common::indent(recovery))
        ),
    ] {
        for cleanup in [
            "target.finish(value)\n",
            "target.value = value\n",
            "with target.manager():\n    pass\n",
            "with target.manager():\n    target.finish(value)\n",
            "for item in items:\n    target.finish(item)\n",
        ] {
            for depth in 0..=2 {
                for named in [false, true] {
                    let name = if named { " as error" } else { "" };
                    let mut body = format!(
                        "try:\n    target.work(value)\nexcept Error{name}:\n{}finally:\n{}",
                        common::indent(&handler),
                        common::indent(cleanup)
                    );
                    for _ in 0..depth {
                        body = format!("with manager():\n{}", common::indent(&body));
                    }
                    body.push_str("target.use(value)\n");
                    assert_matches_cpython(&format!(
                        "def f(target, value, flag, other, items):\n{}",
                        common::indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn guarded_recovery_in_normal_finalizers_keeps_only_the_hot_copy_reachable() {
    let recovery = "try:\n    pass\nexcept NestedError:\n    target.recover(value)\n";
    for handler in [
        recovery.to_owned(),
        format!("if flag:\n{}", common::indent(recovery)),
        format!(
            "if flag:\n    if other:\n        pass\n    else:\n{}",
            common::indent(&common::indent(recovery))
        ),
    ] {
        for work in ["target.work(value)", "result = target.work(value)"] {
            for named in [false, true] {
                for suffix in [
                    "target.suffix(value)\n",
                    "try:\n    result = target.suffix(value)\nexcept Error as error:\n    pass\n",
                ] {
                    let name = if named { " as error" } else { "" };
                    let recovery = format!(
                        "try:\n    {work}\nexcept Error{name}:\n{}{suffix}",
                        common::indent(&handler)
                    );
                    for body in [
                        recovery.clone(),
                        format!("with manager():\n{}", common::indent(&recovery)),
                        format!(
                            "try:\n    target.begin(value)\nfinally:\n{}",
                            common::indent(&recovery)
                        ),
                        format!(
                            "try:\n    target.begin(value)\nfinally:\n    with manager():\n{}",
                            common::indent(&common::indent(&recovery))
                        ),
                        format!(
                            "try:\n    pass\nexcept Error as error:\n    pass\nfinally:\n    with manager():\n{}",
                            common::indent(&common::indent(&recovery))
                        ),
                        format!(
                            "try:\n    target.begin(value)\nexcept Error:\n    with manager():\n{}",
                            common::indent(&common::indent(&recovery))
                        ),
                    ] {
                        assert_matches_cpython(&format!(
                            "def f(target, value, flag, other):\n{}",
                            common::indent(&body)
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn completed_boolean_stores_preserve_live_recovery_cleanup() {
    for value in [
        "value or error",
        "value and error",
        "value or other or error",
        "value and other and error",
    ] {
        for placement in 0..4 {
            let assignment = format!("saved = {value}\n");
            let protected = if placement == 2 {
                assignment.as_str()
            } else {
                "pass\n"
            };
            let recovery_prefix = if placement == 3 { &assignment } else { "" };
            let recovery = format!(
                "try:\n{}except NestedError:\n{}    target.recover(value)\n",
                common::indent(protected),
                common::indent(recovery_prefix)
            );
            let handler_prefix = if placement == 0 { &assignment } else { "" };
            let guard_prefix = if placement == 1 { &assignment } else { "" };
            let handler = format!(
                "{handler_prefix}if flag:\n{}{}",
                common::indent(guard_prefix),
                common::indent(&recovery)
            );
            let recovery = format!(
                "try:\n    result = target.work(value)\nexcept Error as error:\n{}target.suffix(value)\n",
                common::indent(&handler)
            );
            for body in [
                recovery.clone(),
                format!(
                    "try:\n    target.begin(value)\nfinally:\n    with manager():\n{}",
                    common::indent(&common::indent(&recovery))
                ),
                format!(
                    "try:\n    target.begin(value)\nexcept Error:\n{}",
                    common::indent(&recovery)
                ),
            ] {
                assert_matches_cpython(&format!(
                    "def f(target, value, flag, other):\n{}",
                    common::indent(&body)
                ));
            }
        }
    }
}

#[test]
fn nonfinal_recovery_keeps_the_managed_finalizer_entry_owned() {
    for named in [false, true] {
        for cleanup in [
            "target.value = value\n",
            "with target.manager():\n    target.finish(value)\n",
            "for item in items:\n    target.finish(item)\n",
        ] {
            let name = if named { " as error" } else { "" };
            let body = format!(
                "try:\n    target.work(value)\nexcept Error{name}:\n    if flag:\n        if other:\n            try:\n                pass\n            except NestedError:\n                target.recover(value)\n        else:\n            pass\nfinally:\n{}target.use(value)\n",
                common::indent(cleanup)
            );
            assert_matches_cpython(&format!(
                "def f(target, value, flag, other, items):\n{}",
                common::indent(&body)
            ));
        }
    }
}
