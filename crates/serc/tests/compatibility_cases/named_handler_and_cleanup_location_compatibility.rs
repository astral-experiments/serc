use crate::common;

#[test]
fn returns_copy_finally_bodies_before_leaving_the_enclosing_handler() {
    for scope in ["ordinary", "coroutine", "generator"] {
        let prefix = if scope == "coroutine" { "async " } else { "" };
        let await_prefix = if scope == "coroutine" { "await " } else { "" };
        let suspension = if scope == "generator" {
            "    yield None\n"
        } else {
            ""
        };
        for handler in ["except Error:", "except Error as error:", "except:"] {
            for value in ["None", "value", "run(value)"] {
                let value = if value == "run(value)" {
                    format!("{await_prefix}{value}")
                } else {
                    value.to_owned()
                };
                for finish in ["", "            finish(None)\n"] {
                    let source = format!(
                        "{prefix}def f(value, flag):\n    try:\n        {await_prefix}first(value)\n    {handler}\n        state = resource()\n        try:\n            prepare(state)\n            return {value}\n        finally:\n            {await_prefix}cleanup()\n{finish}{suspension}",
                    );
                    common::assert_matches_cpython_marshal(&source, "handler_finally_return.py");
                }
            }
        }
    }

    let source = "def f(value):\n    try:\n        return work(value)\n    except Error:\n        state = resource()\n        try:\n            prepare(state)\n            return run(state, value)\n        finally:\n            state.close()\n            finish(None)\n";
    common::assert_matches_cpython_marshal(source, "handler_resource_finally_return.py");
}

#[test]
fn copied_finally_returns_discard_pending_values_before_outer_cleanup() {
    for binding in ["", " as error"] {
        for original in ["None", "value"] {
            for replacement in ["None", "value"] {
                let source = format!(
                    "def f(value, flag):\n    try:\n        work()\n    except Error{binding}:\n        try:\n            return {original}\n        finally:\n            if flag:\n                return {replacement}\n",
                );
                common::assert_matches_cpython_marshal(&source, "pending_handler_return.py");
            }
        }
    }

    for scope in ["ordinary", "coroutine", "async_with"] {
        let prefix = if scope == "ordinary" { "" } else { "async " };
        let initial = if scope == "ordinary" {
            ""
        } else {
            "    await initial()\n"
        };
        let manager_prefix = if scope == "async_with" { "async " } else { "" };
        for binding in ["", " as error"] {
            for value in ["None", "value", "run(value)"] {
                for conditional in [
                    "",
                    "                    if flag:\n                        return other\n",
                ] {
                    let source = format!(
                        "{prefix}def f(value, other, flag, values, manager, second_manager):\n{initial}    try:\n        first()\n    except Error{binding}:\n        for item in values:\n            {manager_prefix}with manager:\n                try:\n                    return {value}\n                finally:\n{conditional}                    cleanup()\n",
                    );
                    common::assert_matches_cpython_marshal(&source, "pending_manager_return.py");
                }
            }
        }
    }
}

#[test]
fn nested_retained_try_entries_use_fresh_truth_marker_slots() {
    let indent = |body: &str| format!("    {}", body.replace('\n', "\n    "));
    for captured in [false, true] {
        for condition in ["flag", "obj.ready"] {
            for prologue in ["", "obj.result = value\n", "obj.status = Status.DONE\n"] {
                for outer_finally in [false, true] {
                    let body = format!(
                        "{prologue}try:\n    if {condition}:\n        self.wait()\nexcept Error as exc:\n    raise\nreturn value\n",
                    );
                    let cleanup = if outer_finally {
                        "finally:\n    release()"
                    } else {
                        "except OuterError:\n    recover()"
                    };
                    let body = format!("try:\n{}\n{cleanup}\n", indent(&body));
                    let source = if captured {
                        let function = format!("def f(value):\n{}\nreturn f", indent(&body));
                        format!("def outer(obj, self, flag):\n{}\n", indent(&function))
                    } else {
                        format!("def f(value, obj, self, flag):\n{}\n", indent(&body))
                    };
                    common::assert_matches_cpython_marshal(&source, "retained_try_entry.py");
                }
            }
        }
    }
}

#[test]
fn handler_returns_keep_manager_and_finally_cleanup_owners() {
    for prefix in ["", "async "] {
        for value in ["None", "value", "call(value)"] {
            for binding in ["", " as error"] {
                for managers in ["first_manager", "first_manager, second_manager"] {
                    let source = format!(
                        "{prefix}def f(value, flag):\n    try:\n        with {managers}:\n            try:\n                work()\n            except Error{binding}:\n                prepare()\n                if flag:\n                    return {value}\n                use()\n        return {value}\n    finally:\n        cleanup()\n",
                    );
                    common::assert_matches_cpython_marshal(&source, "handler_manager_return.py");
                }
            }
        }
    }
}

#[test]
fn normal_handler_cleanup_keeps_intervening_protected_regions() {
    for prefix in ["", "async "] {
        for body in [
            "try:\n    cancel()\nexcept Error:\n    pass\nfinally:\n    close()\nraise",
            "try:\n    raise outer_error\nfinally:\n    try:\n        refresh()\n    except Error as inner_error:\n        report(inner_error)",
        ] {
            let source = format!(
                "{prefix}def f():\n    try:\n        work()\n    except Error as outer_error:\n        {}\n",
                body.replace('\n', "\n        "),
            );
            common::assert_matches_cpython_marshal(&source, "intervening_handler_cleanup.py");
        }
    }
}

#[test]
fn nested_named_handler_returns_keep_the_outer_name_cleanup() {
    for prefix in ["", "async "] {
        for outer_binding in ["", " as outer_error"] {
            for value in ["None", "value", "call(value)"] {
                let source = format!(
                    "{prefix}def f(value):\n    try:\n        work()\n    except Error{outer_binding}:\n        try:\n            work()\n        except Error as inner_error:\n            return {value}\n"
                );
                common::assert_matches_cpython_marshal(&source, "nested_named_handler_return.py");
            }
        }
    }

    for manager in ["", "            with manager:\n"] {
        let body = if manager.is_empty() {
            "            try:\n                work()\n            except Error as inner_error:\n                return value\n            work()\n"
        } else {
            "                try:\n                    work()\n                except Error as inner_error:\n                    return value\n                work()\n"
        };
        let source = format!(
            "def f(value, manager):\n    try:\n        work()\n    except Error as outer_error:\n        try:\n            work()\n        except Error as middle_error:\n{manager}{body}        work()\n    work()\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_named_handler_manager_return.py");
    }

    for prefix in ["", "async "] {
        let source = format!(
            "{prefix}def f(value):\n    try:\n        work()\n    except Error as outer_error:\n        try:\n            work()\n        except Error as middle_error:\n            try:\n                work()\n            except Error as inner_error:\n                return value\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_named_handler_implicit_return.py");
        let source = format!("{source}    finish()\n");
        common::assert_matches_cpython_marshal(&source, "nested_named_handler_normal_exit.py");
    }
}

#[test]
fn named_handler_return_clear_keeps_the_remaining_finally_owner() {
    for scope in ["ordinary", "coroutine", "generator", "async_generator"] {
        let prefix = if matches!(scope, "coroutine" | "async_generator") {
            "async "
        } else {
            ""
        };
        let suspension = if matches!(scope, "generator" | "async_generator") {
            "    yield None\n"
        } else {
            ""
        };
        for value in ["", " None", " value", " call(value)"] {
            if scope == "async_generator" && !value.is_empty() {
                continue;
            }
            let source = format!(
                "{prefix}def f(value, flag):\n{suspension}    try:\n        work()\n    except Error as error:\n        if flag:\n            return{value}\n        work()\n    finally:\n        cleanup()\n"
            );
            common::assert_matches_cpython_marshal(&source, "named_handler_return_clear.py");
            if scope == "ordinary" {
                let source =
                    source.replace("        if flag:\n            return", "        return");
                common::assert_matches_cpython_marshal(
                    &source,
                    "direct_named_handler_return_clear.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_named_handler_and_cleanup_locations() {
    let cases = [
        (
            "named_and_raise.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready:
            raise
    finish(value)
",
        ),
        (
            "named_and_three_raise.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready and error.again:
            raise
    finish(value)
",
        ),
        (
            "named_and_return.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready:
            return
    finish(value)
",
        ),
        (
            "named_and_three_return.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready and error.again:
            return
    finish(value)
",
        ),
        (
            "unnamed_and_raise_control.py",
            r"
def check(value):
    try:
        use(value)
    except Error:
        if value.bad and value.ready:
            raise
    finish(value)
",
        ),
        (
            "named_or_raise_control.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad or error.ready:
            raise
    finish(value)
",
        ),
        (
            "named_single_raise_control.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad:
            raise
    finish(value)
",
        ),
        (
            "named_and_nonterminal_control.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready:
            consume(error)
    finish(value)
",
        ),
        (
            "named_and_followed_in_handler_control.py",
            r"
def check(value):
    try:
        use(value)
    except Error as error:
        if error.bad and error.ready:
            raise
        consume(error)
    finish(value)
",
        ),
    ];

    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_exceptional_return_manager_cleanup() {
    for source in [
        r"def f(xs, flag):
    count = 0
    with make() as conn:
        try:
            work(conn)
        finally:
            return
    pause()
",
        r"def f(xs, flag):
    count = 0
    with make(), make() as conn:
        try:
            work(conn)
        finally:
            return count
    pause()
",
        r"def f(xs, flag):
    count = 0
    with make(), make() as conn:
        try:
            work(conn)
        except Error as error:
            work(error)
        finally:
            return work(count)
    pause()
",
        r"def f(xs, flag):
    count = 0
    with make(), make() as conn:
        try:
            work(conn)
        except* Error as error:
            work(error)
        finally:
            work(count)
            return count
    pause()
",
        r"def f(xs, flag):
    count = 0
    with make(), make() as conn:
        try:
            work(conn)
        except Error:
            work(count)
        else:
            work(flag)
        finally:
            return work(count)
    pause()
",
    ] {
        common::assert_matches_cpython_marshal(source, "exceptional_return_cleanup.py");
    }
}

#[test]
fn matches_cpython_exceptional_return_async_manager_cleanup() {
    for source in [
        r"async def f(flag):
    count = 0
    async with amake():
        with make() as conn:
            try:
                work(conn)
            finally:
                return await awork(count)
    pause()
",
        r"async def f(flag):
    count = 0
    async with amake(), amake() as conn:
        try:
            work(conn)
        except Error as error:
            work(error)
        finally:
            return work(count)
    pause()
",
        r"async def f(flag):
    count = 0
    async with amake():
        with make() as conn:
            try:
                work(conn)
            except* Error as error:
                work(error)
            finally:
                return 0
    pause()
",
        r"async def f(flag):
    count = 0
    yield count
    with make():
        async with amake() as conn:
            try:
                work(conn)
            finally:
                return
    pause()
",
    ] {
        common::assert_matches_cpython_marshal(source, "exceptional_return_cleanup.py");
    }
}

#[test]
fn preserves_cpython_exceptional_bare_return_without_managers() {
    for source in [
        r"def f(xs, flag):
    count = 0
    conn = flag
    try:
        work(conn)
    finally:
        return
    pause()
",
        r"def f(xs, flag):
    count = 0
    conn = flag
    try:
        work(conn)
    except* Error as error:
        work(error)
    finally:
        return
    pause()
",
    ] {
        common::assert_matches_cpython_marshal(source, "exceptional_return_cleanup.py");
    }
}

#[test]
fn matches_cpython_finally_return_overrides_loop_exit() {
    for source in [
        r"def f(xs, flag):
    count = 0
    conn = flag
    while True:
        try:
            break
        finally:
            return count
    if count:
        work(count)
    pause()
",
        r"def f(xs, flag):
    count = 0
    with make() as conn:
        while True:
            try:
                break
            finally:
                return count
    if count:
        work(count)
    pause()
",
        r"def f(xs, flag):
    count = 0
    with make(), make() as conn:
        for item in xs:
            try:
                continue
            finally:
                return count
    if count:
        work(count)
    pause()
",
        r"def f(xs, flag):
    count = 0
    with make() as conn:
        while True:
            try:
                work(conn)
            except:
                break
            finally:
                return count
    if count:
        work(count)
    pause()
",
        r"def f(xs, flag):
    count = 0
    conn = flag
    for item in xs:
        try:
            work(conn)
        except Error as error:
            break
        finally:
            return count
    if count:
        work(count)
    pause()
",
        r"def f(xs, flag):
    count = 0
    with make() as conn:
        while True:
            try:
                work(conn)
            except Error:
                work(count)
            else:
                break
            finally:
                return count
    if count:
        work(count)
    pause()
",
    ] {
        common::assert_matches_cpython_marshal(source, "finally_loop_return_cleanup.py");
    }
}

#[test]
fn matches_cpython_finally_return_overrides_async_loop_exit() {
    for source in [
        r"async def f(xs, flag):
    value = flag
    async with manager():
        async for item in xs:
            try:
                work(item)
            except Error:
                break
            finally:
                return await finish(value)
    finish_after()
",
        r"async def f(xs, flag):
    value = flag
    async with manager():
        async for item in xs:
            try:
                work(item)
            except Error:
                continue
            finally:
                return value
    finish_after()
",
        r"async def f(xs, flag):
    value = flag
    async with manager():
        async for item in xs:
            try:
                work(item)
                continue
            finally:
                return value
    finish_after()
",
        r"async def f(xs, flag):
    value = flag
    async with manager():
        async for item in xs:
            try:
                work(item)
                yield item
            except Error:
                break
            finally:
                return
    finish_after()
",
    ] {
        common::assert_matches_cpython_marshal(source, "finally_loop_return_cleanup.py");
    }
}

#[test]
fn keeps_overriding_loop_return_after_unregistered_nested_finalizer() {
    use std::io::Write as _;
    use std::process::Stdio;

    // Later nested-finalizer copies still have inherited bytecode differences.
    // This regression checks the native return on the normal loop-exit path:
    // a stale inner-finalizer counter used to discard its value and fall through.
    // Whole-marshal tests and corpus identity checks remain unchanged.
    let source = r"def f(xs, flag):
    count = 0
    conn = flag
    while True:
        try:
            try:
                break
            finally:
                return count
        finally:
            return count + 1
    if count:
        work(count)
    pause()
";
    let oracle = r#"
import dis, marshal, sys
actual = marshal.loads(sys.stdin.buffer.read()).co_consts[0]
expected = compile(sys.argv[1], sys.argv[2], "exec", dont_inherit=True, optimize=0).co_consts[0]
def normal_returns(code):
    result = []
    for instruction in dis.get_instructions(code):
        if instruction.opname == "PUSH_EXC_INFO":
            break
        if instruction.opname == "RETURN_VALUE":
            result.append((code.co_code[instruction.offset:instruction.offset + 2], tuple(instruction.positions)))
    return result
wanted = [(bytes([dis.opmap["RETURN_VALUE"], 0]), (11, 11, 12, 28))]
assert normal_returns(expected) == wanted
assert normal_returns(actual) == wanted
"#;
    let filename = "nested_finally_loop_return.py";
    let actual = serc::compile(source, filename).unwrap().marshal();
    let mut child = common::python_3145_command()
        .args(["-c", oracle, source, filename])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&actual).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "overriding loop-exit return differs from CPython: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn matches_cpython_copied_loop_return_retires_inner_collectors() {
    for source in [
        r"def f(xs, flag):
    count = 0
    conn = flag
    try:
        work(conn)
    except Error:
        while True:
            try:
                break
            finally:
                return count
    pause()
",
        r"def f(xs, flag):
    count = 0
    conn = flag
    try:
        work(conn)
    except Error:
        while flag:
            try:
                break
            finally:
                return count
    pause()
",
    ] {
        common::assert_matches_cpython_marshal(source, "copied_loop_return_collectors.py");
    }
}

#[test]
fn matches_cpython_literal_return_discards_iterator_inside_handler() {
    for source in [
        r"def f(xs, flag):
    count = 0
    try:
        work(flag)
    except Error:
        for item in xs:
            try:
                break
            finally:
                return None
    pause()
",
        r"def f(xs, flag):
    count = 0
    try:
        work(flag)
    except:
        for item in xs:
            try:
                break
            finally:
                return None
    pause()
",
        r"def f(xs, flag):
    count = 0
    try:
        work(flag)
    except Error as error:
        for item in xs:
            try:
                break
            finally:
                return None
    pause()
",
        r"def f(xs, flag):
    count = 0
    try:
        work(flag)
    except Error:
        for item in xs:
            return None
    pause()
",
        r"def f(xs, flag):
    count = 0
    try:
        work(flag)
    except:
        for item in xs:
            return None
    pause()
",
        r"def f(xs, flag):
    count = 0
    try:
        work(flag)
    except Error as error:
        for item in xs:
            return None
    pause()
",
    ] {
        common::assert_matches_cpython_marshal(source, "literal_return_iterator_owner.py");
    }
}

#[test]
fn matches_cpython_handler_iterator_return_before_enclosing_finalizer() {
    for source in [
        r"def f(items, flag, value):
    try:
        work()
    except Error as error:
        for item in items:
            try:
                if flag:
                    return
            except InnerError:
                recover()
    finally:
        cleanup()
    return value
",
        r"def f(items, flag, value):
    try:
        work()
    except Error:
        for item in items:
            try:
                if flag:
                    return None
            except InnerError:
                recover()
    finally:
        if flag:
            cleanup()
    return value
",
        r"def f(items, flag, value):
    try:
        work()
    except:
        for item in items:
            try:
                return value
            except InnerError:
                recover()
    finally:
        if flag:
            cleanup()
    return value
",
        r"async def f(items, flag, value):
    try:
        work()
    except Error as error:
        for item in items:
            try:
                return value
            except InnerError:
                recover()
    finally:
        cleanup()
    return value
",
        r"async def f(items, flag, value):
    try:
        work()
    except:
        for item in items:
            try:
                return
            except InnerError:
                recover()
    finally:
        cleanup()
    return value
",
        r"async def f(items, flag, value):
    try:
        work()
    except Error:
        for item in items:
            try:
                if flag:
                    return value
            except InnerError:
                recover()
    finally:
        if flag:
            cleanup()
    return value
",
    ] {
        common::assert_matches_cpython_marshal(source, "handler_iterator_outer_finally.py");
    }
}

#[test]
fn matches_cpython_outer_iterator_return_after_enclosing_finalizer() {
    for source in [
        r"def f(items, flag, value):
    for outer in items:
        try:
            try:
                try:
                    work()
                except Error:
                    for item in items:
                        try:
                            return None
                        except InnerError:
                            recover()
            finally:
                cleanup()
        except OuterError:
            recover()
    return value
",
        r"def f(items, flag, value):
    for outer in items:
        try:
            try:
                try:
                    work()
                except Error as error:
                    for item in items:
                        try:
                            if flag:
                                return value
                        except InnerError:
                            recover()
            finally:
                cleanup()
        except OuterError:
            recover()
    return value
",
    ] {
        common::assert_matches_cpython_marshal(source, "outer_iterator_after_finally.py");
    }
}

#[test]
fn preserves_native_outer_iterator_owners_before_enclosing_finalizer() {
    use std::io::Write as _;
    use std::process::Stdio;

    // These cases retain inherited POP_EXCEPT/name-clear owner differences.
    // Check the complete native code and locations, plus the iterator cleanup
    // words that previously retained the inner ordinary try's handler.
    let oracle = r#"
import dis, marshal, sys
actual = marshal.loads(sys.stdin.buffer.read()).co_consts[0]
expected = compile(sys.argv[1], sys.argv[2], "exec", dont_inherit=True, optimize=0).co_consts[0]
offsets = tuple(map(int, sys.argv[3].split(",")))
assert actual.co_code == expected.co_code
assert actual.co_linetable == expected.co_linetable
def selected_owners(code):
    instructions = {i.offset: i for i in dis.get_instructions(code)}
    entries = dis.Bytecode(code).exception_entries
    result = []
    for offset in offsets:
        instruction = instructions[offset]
        assert instruction.opname in ("POP_TOP", "SWAP")
        owners = [(e.target, e.depth, e.lasti) for e in entries if e.start <= offset < e.end]
        assert len(owners) == 1
        result.append((offset, code.co_code[offset:offset + 2], tuple(instruction.positions), owners[0]))
    return result
assert selected_owners(actual) == selected_owners(expected)
"#;
    for (source, offsets) in [
        (
            r"def f(items, flag, value):
    try:
        for outer in items:
            try:
                try:
                    work()
                except Error:
                    for item in items:
                        try:
                            return None
                        except InnerError:
                            recover()
            except OuterError:
                recover()
    finally:
        cleanup()
    return value
",
            "104,108",
        ),
        (
            r"def f(items, flag, value):
    try:
        for outer in items:
            try:
                try:
                    work()
                except Error as error:
                    for item in items:
                        try:
                            if flag:
                                return value
                        except InnerError:
                            recover()
            except OuterError:
                recover()
    finally:
        cleanup()
    return value
",
            "122,124,136,138",
        ),
    ] {
        let filename = "outer_iterator_before_finally.py";
        let actual = serc::compile(source, filename).unwrap().marshal();
        let mut child = common::python_3145_command()
            .args(["-c", oracle, source, filename, offsets])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&actual).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "outer iterator cleanup owner differs from CPython: {}",
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

#[test]
fn matches_cpython_iterator_return_owners_with_surrounding_managers() {
    for source in [
        r"def f(items, manager):
    with manager:
        try:
            for item in items:
                try:
                    return work(item)
                except Error:
                    recover(item)
        finally:
            finish()
",
        r"async def f(items, outer, inner, state):
    async with outer:
        with inner:
            try:
                for item in items:
                    try:
                        await work(item)
                        return state.value
                    except Error:
                        recover(item)
            finally:
                finish()
",
        r"def f(items, manager):
    with manager:
        try:
            for item in items:
                try:
                    check(item)
                    return None
                except Error:
                    recover(item)
        finally:
            finish()
",
        r"def f(items, other, manager):
    with manager:
        try:
            for item in items:
                for value in other:
                    try:
                        return work(item, value)
                    except Error:
                        recover(value)
        finally:
            finish()
",
        r"def f(items):
    try:
        for item in items:
            try:
                return work(item)
            except Error:
                recover(item)
    finally:
        finish()
",
        r"def f(items, manager):
    try:
        with manager:
            for item in items:
                return work(item)
    finally:
        finish()
",
        r"def f(items, manager):
    with manager:
        try:
            for item in items:
                try:
                    work(item)
                except Error:
                    return item
        finally:
            finish()
",
        r"def f(items, manager):
    for item in items:
        with manager:
            try:
                return work(item)
            finally:
                finish()
",
    ] {
        common::assert_matches_cpython_marshal(source, "iterator_return_manager_owners.py");
    }
}
