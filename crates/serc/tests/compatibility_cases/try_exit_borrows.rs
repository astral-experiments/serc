use crate::common;

#[test]
fn matches_cpython_completed_handler_for_borrowing() {
    for protected_loop in [false, true] {
        let protected = if protected_loop {
            "for item in items:\n    work(value)"
        } else {
            "work(value)"
        };
        for tail in [
            "work(item)",
            "if flag:\n    work(item)",
            "with manager:\n    work(item)",
            "try:\n    work(item)\nexcept OtherError:\n    pass",
        ] {
            for following_loop in [false, true] {
                let following = if following_loop {
                    "for item in items:\n    use(value, item)"
                } else {
                    "use(value)"
                };
                let source = format!(
                    "def f(items, value, flag, manager):\n    try:\n        {}\n    except Error:\n        for item in items:\n            {}\n    {}\n",
                    protected.replace('\n', "\n        "),
                    tail.replace('\n', "\n            "),
                    following.replace('\n', "\n    "),
                );
                common::assert_matches_cpython_marshal(&source, "completed_handler_for.py");
            }
        }
    }
}

#[test]
fn matches_cpython_complete_conditional_normal_finally_borrows() {
    for scope in 0..4 {
        let is_async = scope == 1 || scope == 3;
        let prefix = if is_async { "async " } else { "" };
        let awaited = if is_async { "await " } else { "" };
        let yielded = if scope >= 2 { "yield " } else { "" };
        let work = format!("{yielded}{awaited}sink.work(left)");
        let other = format!("{yielded}{awaited}sink.work(right)");
        let tail = if scope == 3 { "return" } else { "return right" };
        for shape in 0..4 {
            let body = match shape {
                0 => format!("if flag:\n    {work}\nelse:\n    {other}"),
                1 => format!(
                    "if flag:\n    {work}\nelif left is right:\n    {other}\nelse:\n    {work}"
                ),
                2 => work.clone(),
                _ => format!("if flag:\n    {work}"),
            };
            for handler in 0..3 {
                if shape >= 2 && handler != 0 {
                    continue;
                }
                let recovery = match handler {
                    0 => "pass".to_owned(),
                    1 => format!("{awaited}sink.recover()"),
                    _ => "left = right".to_owned(),
                };
                for assigned_cleanup in [false, true] {
                    let cleanup = if assigned_cleanup {
                        format!("result = sink.clock()\n{awaited}sink.close(result)")
                    } else {
                        format!("{awaited}sink.close(left)")
                    };
                    let source = format!(
                        "{prefix}def function(flag, left, right, sink, problem):\n    try:\n        {}\n    except problem:\n        {recovery}\n    finally:\n        {}\n    {tail}\n",
                        body.replace('\n', "\n        "),
                        cleanup.replace('\n', "\n        ")
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "complete_conditional_normal_finally.py",
                    );
                }
            }
        }
    }
}

#[test]
fn preserves_cpython_folded_assert_conditional_finally_boundaries() {
    // The earlier folded assertion leaves empty original blocks before its exit.
    // A later folded assertion, pass, or folded expression does not supply a live entry.
    for scope in 0..4 {
        let is_async = scope == 1 || scope == 3;
        let prefix = if is_async { "async " } else { "" };
        let yielded = if scope >= 2 { "    yield result\n" } else { "" };
        let awaited = if is_async {
            "    await async_tick(y)\n"
        } else {
            ""
        };
        let cleanup = if is_async {
            "await async_sink(x, y, result)"
        } else {
            "sink(x, y, result)"
        };
        for otherwise in ["assert True", "pass", "result = x if False else y"] {
            let source = format!(
                "{prefix}def case(x, y, flag, other):\n    result = x\n{yielded}{awaited}    try:\n        result = touch(x)\n        if flag:\n            assert True\n        else:\n            {otherwise}\n    except ValueError:\n        pass\n    finally:\n        {cleanup}\n    sink(x, result)\n",
            );
            common::assert_matches_cpython_marshal(&source, "folded_assert_conditional_finally.py");
        }
    }
}

#[test]
fn preserves_cpython_unnamed_runtime_while_else_boundaries() {
    for scope in 0..4 {
        let prefix = if scope == 1 || scope == 3 {
            "async "
        } else {
            ""
        };
        let tail = if scope >= 2 {
            "yield value"
        } else {
            "return value"
        };
        for otherwise in [
            "if flag:\n    consume(value)",
            "with manager:\n    consume(value)",
            "try:\n    consume(value)\nexcept Other:\n    consume(other)",
        ] {
            for handler in ["pass", "sink.recover(value)"] {
                let source = format!(
                    "{prefix}def function(source, sink, flag, value, other, manager, values):\n    try:\n        while flag:\n            consume(value)\n            flag = source.take()\n        else:\n            {}\n    except Problem:\n        {handler}\n    finally:\n        sink.close(value)\n    consume(value, other)\n    {tail}\n",
                    otherwise.replace('\n', "\n            ")
                );
                common::assert_matches_cpython_marshal(&source, "runtime_while_else_boundary.py");
            }
        }
    }
}

#[test]
fn matches_cpython_unnamed_runtime_while_finally_borrows() {
    for scope in 0..4 {
        let is_async = scope == 1 || scope == 3;
        let prefix = if is_async { "async " } else { "" };
        let take = if is_async {
            "await source.take()"
        } else {
            "source.take()"
        };
        let yielded = if scope >= 2 { "yield item\n" } else { "" };
        let flushed = if is_async { "await " } else { "" };
        let work = format!("sink.write(item)\n{yielded}{flushed}sink.flush()");
        let tail = if scope == 3 { "return" } else { "return flag" };
        for named in [false, true] {
            let loop_body = if named {
                format!(
                    "while item := {take}:\n    {}",
                    work.replace('\n', "\n    ")
                )
            } else {
                format!(
                    "while flag:\n    item = {take}\n    {}\n    flag = item",
                    work.replace('\n', "\n    ")
                )
            };
            for handler in ["pass", "sink.recover()", "flag = False"] {
                for cleanup in ["sink.close()", "sink.close()\n        sink.finish()"] {
                    let source = format!(
                        "{prefix}def function(source, sink, flag):\n    try:\n        {}\n    except Problem:\n        {handler}\n    finally:\n        {cleanup}\n    {tail}\n",
                        loop_body.replace('\n', "\n        ")
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "unnamed_runtime_while_finally.py",
                    );
                }
            }
        }
    }
}

#[test]
fn preserves_cpython_unnamed_runtime_while_exclusion_controls() {
    for scope in 0..4 {
        let is_async = scope == 1 || scope == 3;
        let prefix = if is_async { "async " } else { "" };
        let entry = if scope >= 2 { "yield value\n    " } else { "" };
        let awaited = if is_async { "await " } else { "" };
        let terminal = if scope == 3 { "return" } else { "return value" };
        for early_return in [false, true] {
            let early = if early_return {
                format!("if flag:\n    {terminal}\n")
            } else {
                String::new()
            };
            let loop_body = format!(
                "{early}sink.write(value)\n{awaited}sink.flush()\nflag = {awaited}source.take()\nif not flag:\n    break"
            );
            let source = format!(
                "{prefix}def function(source, sink, flag, value, manager):\n    {entry}try:\n        while True:\n            {}\n    except Problem:\n        pass\n    finally:\n        sink.close()\n    consume(value, sink)\n    {terminal}\n",
                loop_body.replace('\n', "\n            ")
            );
            common::assert_matches_cpython_marshal(&source, "constant_while_finally_control.py");
        }
        for loop_exit in ["continue", terminal] {
            let source = format!(
                "{prefix}def function(source, sink, flag, value, manager):\n    {entry}try:\n        while flag:\n            sink.write(value)\n            {awaited}sink.flush()\n            flag = {awaited}source.take()\n            if flag:\n                {loop_exit}\n{}    finally:\n        sink.close()\n    consume(value, sink)\n    {terminal}\n",
                if loop_exit == "continue" {
                    "            sink.write(value)\n"
                } else {
                    ""
                }
            );
            common::assert_matches_cpython_marshal(&source, "no_handler_runtime_while_control.py");
        }
    }
}

#[test]
fn matches_cpython_borrows_before_separately_compiled_loop_tail_conditions() {
    for prefix in ["", "async "] {
        for loop_header in ["for item in items", "while flag"] {
            for body in [
                "if other:\n    work(value)",
                "with manager():\n    work(value)",
                "try:\n    work(value)\nexcept OtherError:\n    pass",
            ] {
                for handler in ["except Error", "except Error as error"] {
                    for tail in ["if value:\n    work(other)", "if value:\n    break"] {
                        let body = body.replace('\n', "\n            ");
                        let tail = tail.replace('\n', "\n        ");
                        let source = format!(
                            "{prefix}def f(value, other, flag, items):\n    {loop_header}:\n        try:\n            {body}\n        {handler}:\n            pass\n        {tail}\n    return value\n"
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "try_exit_before_loop_tail.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_after_handler_cleanup_orphans() {
    for prefix in ["", "async "] {
        for in_loop in [false, true] {
            for handler in [
                "with manager():\n    work(other)",
                "with manager():\n    pass",
                "with manager():\n    work(other)\nwork(value)",
                "with manager():\n    work(other)\nresult = value",
                "result = tuple(item.attr for item in items)",
                "result = tuple(item.attr for item in items)\nwork(other)",
                "result = lambda: tuple(item.attr for item in items)",
                "result = (tuple(item.attr for item in items) for entry in other)",
                "result = tuple(item.attr for item in items) if False else other",
                "result = other if True else tuple(item.attr for item in items)",
                "result = False and tuple(item.attr for item in items)",
                "result = True or tuple(item.attr for item in items)",
            ] {
                let handler = handler.replace('\n', "\n    ");
                let body = format!(
                    "try:\n    work(value)\nexcept Error as error:\n    {handler}\nwork(value)\nreturn value"
                );
                let body = if in_loop {
                    format!("for entry in items:\n    {}", body.replace('\n', "\n    "))
                } else {
                    body
                };
                let source = format!(
                    "{prefix}def f(value, other, flag, items):\n    {}\n",
                    body.replace('\n', "\n    ")
                );
                common::assert_matches_cpython_marshal(&source, "handler_cleanup_borrows.py");
            }
        }
    }
}

#[test]
fn ignores_unevaluated_handler_annotations_aliases_and_assert_messages() {
    for future in ["", "from __future__ import annotations\n"] {
        for handler in [
            "result: tuple(item.attr for item in items) = other",
            "value.attr: tuple(item.attr for item in items)",
            "value.attr: tuple(item.attr for item in items) = other",
            "type Result = tuple(item.attr for item in items)",
            "type Result = list[tuple(item.attr for item in items)]",
            "assert True, tuple(item.attr for item in items)",
        ] {
            let source = format!(
                "{future}def f(value, other, items):\n    try:\n        work(value)\n    except Error as error:\n        {handler}\n    work(value)\n    return value\n"
            );
            common::assert_matches_cpython_marshal(&source, "unevaluated_handler_expressions.py");
        }
    }
}

#[test]
fn matches_cpython_borrows_after_async_handler_manager_cleanup() {
    for body in ["work(value)", "await work(value)"] {
        for handler in ["pass", "await work(other)"] {
            for name in ["", " as error"] {
                let source = format!(
                    "async def f(value, other):\n    try:\n        {body}\n    except Error{name}:\n        async with manager():\n            {handler}\n    work(value)\n    return value\n"
                );
                common::assert_matches_cpython_marshal(&source, "async_handler_cleanup_borrows.py");
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_after_partial_terminal_handler_branches() {
    for prefix in ["", "async "] {
        for name in ["", " as error"] {
            for branch in [
                "if condition:\n    value = None\nelse:\n    raise",
                "if condition:\n    raise\nelse:\n    value = None",
                "if condition:\n    raise\nvalue = None",
                "if condition:\n    raise\nelse:\n    value = None\nconsume(condition)",
                "if condition:\n    value = None\nelse:\n    raise\nconsume(condition)",
            ] {
                for tail in ["return consume(value)", "return value"] {
                    let branch = branch.replace('\n', "\n        ");
                    let source = format!(
                        "{prefix}def f(factory, condition, consume):\n    try:\n        value = factory()\n    except ValueError{name}:\n        {branch}\n    {tail}\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "partial_terminal_handler_borrows.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_after_incomplete_handler_branches() {
    for prefix in ["", "async "] {
        for name in ["", " as error"] {
            for branch in [
                "if condition:\n    raise\nelif other:\n    value = None\nconsume(condition)",
                "if condition:\n    value = None\nelif other:\n    raise\nconsume(condition)",
            ] {
                for tail in ["return consume(value)", "return value"] {
                    let branch = branch.replace('\n', "\n        ");
                    let source = format!(
                        "{prefix}def f(value, factory, condition, other, consume):\n    try:\n        value = factory()\n    except ValueError{name}:\n        {branch}\n    {tail}\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "incomplete_handler_borrows.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_after_recovery_handler_else_returns() {
    for (prefix, leading) in [("", ""), ("async ", ""), ("", "    yield resource\n")] {
        for handler in [
            "value = None",
            "consume(resource)",
            "if flag:\n    consume(resource)",
            "if flag:\n    return resource",
            "assert flag",
        ] {
            for name in ["", " as error"] {
                for condition in ["flag", "flag or other"] {
                    let source = format!(
                        "{prefix}def f(resource, receive, consume, value, flag, other, manager, items):\n{leading}    try:\n        value = receive(resource)\n    except Error{name}:\n        {}\n    else:\n        if {condition}:\n            return value\n    consume(resource)\n",
                        handler.replace('\n', "\n        "),
                    );
                    common::assert_matches_cpython_marshal(&source, "recovery_handler_else.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_after_nested_try_else_exit_nops() {
    for (prefix, leading, protected) in [
        ("", "", "work(value)"),
        ("async ", "", "await work(value)"),
        ("", "    yield value\n", "work(value)"),
    ] {
        for branch in [
            "",
            "        if flag:\n            raise Error\n",
            "        if flag:\n            consume(other)\n",
        ] {
            for terminal in ["raise Error", "return result"] {
                for following in [
                    "return consume(value, other)",
                    "try:\n    consume(value, other)\nexcept Error:\n    pass",
                ] {
                    let source = format!(
                        "{prefix}def function(value, other, flag, work, consume):\n{leading}    try:\n{branch}        try:\n            result = {protected}\n        except Error:\n            consume(other)\n        else:\n            if result:\n                {terminal}\n    except Error:\n        pass\n    {}\n",
                        following.replace('\n', "\n    "),
                    );
                    common::assert_matches_cpython_marshal(&source, "nested_try_else_exit_nops.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_through_nested_try_else_bypasses() {
    for (prefix, leading, protected) in [
        ("", "", "work(value)"),
        ("async ", "", "await work(value)"),
        ("", "    yield value\n", "work(value)"),
    ] {
        for following in [
            "return value",
            "return consume(value, other)",
            "try:\n    consume(value, other)\nexcept Error:\n    pass",
        ] {
            let source = format!(
                "{prefix}def function(value, other, flag, work, consume):\n{leading}    try:\n        if flag:\n            raise Error\n        try:\n            result = {protected}\n        except Error:\n            consume(other)\n        else:\n            if result:\n                consume(value)\n            else:\n                consume(other)\n    except Error:\n        pass\n    {}\n",
                following.replace('\n', "\n    "),
            );
            common::assert_matches_cpython_marshal(&source, "nested_try_else_bypass.py");
        }
    }
}

#[test]
fn matches_cpython_borrows_through_folded_nested_try_else_bypasses() {
    // Constant wrappers have separate existing NOP differences. Check their continuation
    // directly against CPython so those differences cannot hide a changed borrowing decision.
    const ORACLE: &str = r#"
import dis, marshal, pathlib, sys, types
source = pathlib.Path(sys.argv[1]).read_text()
ours = marshal.loads(pathlib.Path(sys.argv[2]).read_bytes())
expected = compile(source, "folded_nested_try_else_bypass.py", "exec", dont_inherit=True)
def continuation(module):
    code = next(value for value in module.co_consts if isinstance(value, types.CodeType))
    return [(instruction.opname, instruction.arg, tuple(instruction.positions))
            for instruction in dis.get_instructions(code)
            if instruction.positions.lineno == len(source.splitlines())]
reference = continuation(expected)
assert reference, "oracle continuation must contain instructions"
assert continuation(ours) == reference, "continuation instruction or location differs"
"#;
    for (prefix, leading, protected) in [
        ("", "", "work(value)"),
        ("async ", "", "await work(value)"),
        ("", "    yield value\n", "work(value)"),
    ] {
        for wrapper in [
            "if True:\n    if result:\n        consume(value)\n    else:\n        consume(other)",
            "if False:\n    pass\nelse:\n    if result:\n        consume(value)\n    else:\n        consume(other)",
            "if True:\n    if True:\n        if result:\n            consume(value)\n        else:\n            consume(other)",
        ] {
            let source = format!(
                "{prefix}def function(value, other, flag, work, consume):\n{leading}    try:\n        if flag:\n            raise Error\n        try:\n            result = {protected}\n        except Error:\n            consume(other)\n        else:\n            {}\n    except Error:\n        pass\n    return consume(value, other)\n",
                wrapper.replace('\n', "\n            "),
            );
            let directory = tempfile::tempdir().unwrap();
            let source_path = directory.path().join("source.py");
            let marshal_path = directory.path().join("ours.marshal");
            std::fs::write(&source_path, &source).unwrap();
            std::fs::write(
                &marshal_path,
                serc::compile(&source, "folded_nested_try_else_bypass.py")
                    .unwrap()
                    .marshal(),
            )
            .unwrap();
            let output = common::python_3145_command()
                .args(["-c", ORACLE])
                .arg(&source_path)
                .arg(&marshal_path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
fn matches_cpython_eager_handler_definition_cleanup_borrows() {
    for (prefix, leading) in [("", ""), ("async ", ""), ("", "    yield resource\n")] {
        for handler in [
            "def inner(arg=tuple(item for item in items)):\n    return arg",
            "@decorate(tuple(item for item in items))\ndef inner():\n    pass",
            "class Inner(make_base(tuple(item for item in items))):\n    pass",
            "@decorate(tuple(item for item in items))\nclass Inner:\n    pass",
            "value = lambda arg=tuple(item for item in items): arg",
            "value = (item for item in tuple(item for item in items))",
        ] {
            for otherwise in [
                "",
                "    else:\n        if flag:\n            return value\n",
            ] {
                let source = format!(
                    "{prefix}def f(resource, receive, consume, value, flag, other, manager, items):\n{leading}    try:\n        value = receive(resource)\n    except Error:\n        {}\n{otherwise}    consume(resource)\n",
                    handler.replace('\n', "\n        "),
                );
                common::assert_matches_cpython_marshal(&source, "eager_handler_definition.py");
            }
        }
    }
}

#[test]
fn matches_cpython_borrow_barriers_after_terminal_or_cleanup_handlers() {
    for (prefix, leading) in [("", ""), ("async ", ""), ("", "    yield resource\n")] {
        for handler in [
            "return resource",
            "raise",
            "with manager:\n    consume(resource)",
        ] {
            let source = format!(
                "{prefix}def f(resource, receive, consume, value, flag, other, manager, items):\n{leading}    try:\n        value = receive(resource)\n    except Error:\n        {}\n    else:\n        if flag:\n            return value\n    consume(resource)\n",
                handler.replace('\n', "\n        "),
            );
            common::assert_matches_cpython_marshal(&source, "recovery_handler_barriers.py");
        }
    }
}

#[test]
fn matches_cpython_borrows_after_complete_handler_recovery_branches() {
    for (prefix, leading) in [("", ""), ("async ", ""), ("", "    yield resource\n")] {
        for exit in ["return resource", "raise ValueError"] {
            for name in ["", " as error"] {
                for post in ["", "        consume(value)\n"] {
                    let source = format!(
                        "{prefix}def f(resource, receive, consume, accepted, value, flag, other):\n{leading}    try:\n        value = receive(resource)\n    except ValueError{name}:\n        if flag:\n            {exit}\n        else:\n            value = resource\n{post}    else:\n        if accepted(value):\n            return value\n    consume(resource)\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "complete_handler_recovery_branches.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_after_direct_nested_recovery() {
    for (prefix, protected) in [
        ("", "value = receive(other)"),
        ("async ", "value = await receive(other)"),
        ("", "value = yield other"),
    ] {
        for recovery in ["None", "resource", "receive(resource)"] {
            for tail in [
                "return value",
                "return (resource, value)",
                "return consume(value)",
                "consume(value)\nreturn resource",
            ] {
                let source = format!(
                    "{prefix}def guarded(resource, other, value, receive, consume):\n    try:\n        value = receive(resource)\n    except LookupError:\n        try:\n            {protected}\n        except ValueError:\n            value = {recovery}\n    {}\n",
                    tail.replace('\n', "\n    "),
                );
                common::assert_matches_cpython_marshal(&source, "direct_nested_recovery.py");
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_after_conditional_nested_recovery() {
    // These conditional joins have existing NOP and table differences. Compare the following
    // return's instructions and positions to the pinned oracle to protect its bypass edge.
    const ORACLE: &str = r#"
import dis, marshal, pathlib, sys, types
source = pathlib.Path(sys.argv[1]).read_text()
ours = marshal.loads(pathlib.Path(sys.argv[2]).read_bytes())
reference = compile(source, "conditional_nested_recovery.py", "exec", dont_inherit=True, optimize=0)
ours = next(value for value in ours.co_consts if isinstance(value, types.CodeType))
reference = next(value for value in reference.co_consts if isinstance(value, types.CodeType))
tail_line = len(source.splitlines())
def tail(code):
    return [(instruction.opname, instruction.arg, tuple(instruction.positions))
            for instruction in dis.get_instructions(code)
            if instruction.positions.lineno == tail_line]
expected = tail(reference)
assert expected, "oracle continuation must contain instructions"
assert tail(ours) == expected, "continuation instruction or location differs"
"#;
    for (prefix, leading) in [("", ""), ("async ", ""), ("", "    yield resource\n")] {
        for condition in [
            "flag",
            "all(item for item in items)",
            "any(item for item in items)",
        ] {
            for name in ["", " as error"] {
                let source = format!(
                    "{prefix}def guarded(resource, other, value, receive, consume, flag, items):\n{leading}    try:\n        value = receive(resource)\n    except LookupError{name}:\n        if {condition}:\n            try:\n                value = receive(other)\n            except ValueError:\n                value = resource\n    return consume(value)\n",
                );
                let directory = tempfile::tempdir().unwrap();
                let source_path = directory.path().join("source.py");
                let marshal_path = directory.path().join("ours.marshal");
                std::fs::write(&source_path, &source).unwrap();
                std::fs::write(
                    &marshal_path,
                    serc::compile(&source, "conditional_nested_recovery.py")
                        .unwrap()
                        .marshal(),
                )
                .unwrap();
                let output = common::python_3145_command()
                    .args(["-c", ORACLE])
                    .arg(&source_path)
                    .arg(&marshal_path)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}

#[test]
fn matches_cpython_located_nested_recovery_nops() {
    // Nested handler exit copying still has separate instruction and table differences here.
    // The protected body's final located NOP must nevertheless survive borrow analysis.
    const ORACLE: &str = r#"
import dis, marshal, pathlib, sys, types
source = pathlib.Path(sys.argv[1]).read_text()
ours = marshal.loads(pathlib.Path(sys.argv[2]).read_bytes())
reference = compile(source, "empty_nested_recovery.py", "exec", dont_inherit=True, optimize=0)
def nops(module):
    code = next(value for value in module.co_consts if isinstance(value, types.CodeType))
    return [tuple(instruction.positions) for instruction in dis.get_instructions(code)
            if instruction.opname == "NOP"]
expected = nops(reference)
assert expected, "oracle must retain located NOPs"
assert nops(ours) == expected, "located NOPs differ"
"#;
    for prefix in ["", "value = resource\n", "value = receive(resource)\n"] {
        for noop in [
            "pass",
            "42",
            "'text'",
            "(1, 2)",
            "None",
            "True",
            "assert True",
        ] {
            // A trailing pass after live code has a separate preexisting missing-NOP issue.
            if !prefix.is_empty() && noop == "pass" {
                continue;
            }
            let protected = format!("{prefix}{noop}").replace('\n', "\n        ");
            for nested in ["pass", "value = other"] {
                let source = format!(
                    "def guarded(resource, other, value, receive, consume):\n    try:\n        {protected}\n    except LookupError:\n        try:\n            {nested}\n        except ValueError:\n            value = resource\n    return consume(value)\n",
                );
                let directory = tempfile::tempdir().unwrap();
                let source_path = directory.path().join("source.py");
                let marshal_path = directory.path().join("ours.marshal");
                std::fs::write(&source_path, &source).unwrap();
                std::fs::write(
                    &marshal_path,
                    serc::compile(&source, "empty_nested_recovery.py")
                        .unwrap()
                        .marshal(),
                )
                .unwrap();
                let output = common::python_3145_command()
                    .args(["-c", ORACLE])
                    .arg(&source_path)
                    .arg(&marshal_path)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}

#[test]
fn matches_cpython_retained_exit_nops_before_nested_recovery() {
    for (prefix, leading) in [("", ""), ("async ", ""), ("", "    yield resource\n")] {
        for protected in [
            "for value in items:\n    if flag:\n        break\n    consume(value)",
            "if flag:\n    value = receive(resource)\nelse:\n    value = other",
        ] {
            for recovery in ["None", "resource"] {
                for tail in ["return value", "return consume(value)"] {
                    let source = format!(
                        "{prefix}def guarded(resource, other, value, receive, consume, flag, items, manager):\n{leading}    try:\n        {}\n    except LookupError:\n        try:\n            value = receive(other)\n        except ValueError:\n            value = {recovery}\n    {tail}\n",
                        protected.replace('\n', "\n        "),
                    );
                    common::assert_matches_cpython_marshal(&source, "retained_nested_recovery.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_conditional_break_into_normal_finally() {
    for prefix in ["", "async "] {
        for handler in ["raise", "return", "pass", "sink(y)"] {
            for cleanup in ["sink(x)", "if x:\n    sink(y)", "y = sink(x)\nsink(y)"] {
                let source = format!(
                    "{prefix}def f(x, y):\n    try:\n        while True:\n            if x:\n                break\n            sink(y)\n    except Exception:\n        {handler}\n    finally:\n        {}\n",
                    cleanup.replace('\n', "\n        "),
                );
                common::assert_matches_cpython_marshal(&source, "conditional_break_finally.py");
            }
        }
    }
}

#[test]
fn preserves_cpython_empty_finally_loop_entry_after_break() {
    for prefix in ["", "async "] {
        for handler in ["raise", "return", "pass", "sink(y)"] {
            let source = format!(
                "{prefix}def f(x, y):\n    try:\n        while True:\n            if x:\n                break\n            sink(y)\n    except Exception:\n        {handler}\n    finally:\n        while x:\n            sink(y)\n",
            );
            common::assert_matches_cpython_marshal(&source, "empty_finally_loop_entry.py");
        }
    }
}

#[test]
fn preserves_cpython_dead_break_finally_boundaries() {
    for body in [
        "if True:\n    sink(y)\nelif x:\n    break\nbreak",
        "if x:\n    return y\n    break\nsink(y)\nbreak",
        "if x:\n    raise RuntimeError\n    break\nsink(y)\nbreak",
        "if x:\n    assert False\n    break\nsink(y)\nbreak",
        "if x and False:\n    break\nsink(y)\nbreak",
        "if x:\n    assert 0\n    break\nsink(y)\nbreak",
    ] {
        let source = format!(
            "def f(x, y):\n    try:\n        while True:\n            {}\n    except Exception:\n        raise\n    finally:\n        sink(x)\n",
            body.replace('\n', "\n            "),
        );
        common::assert_matches_cpython_marshal(&source, "dead_break_finally.py");
    }
}

#[test]
fn preserves_bare_handler_normal_exit_borrow_boundaries() {
    for (prefix, leading) in [("", ""), ("async ", ""), ("", "    yield value\n")] {
        for handler in ["", " Exception", " Exception as error"] {
            for recovery in ["pass", "result = None"] {
                let guarded =
                    format!("try:\n    result = action(value)\nexcept{handler}:\n    {recovery}");
                for body in [
                    format!(
                        "for item in items:\n    {}\n    if result:\n        consume(value)\n        continue\n    else:\n        consume(result)",
                        guarded.replace('\n', "\n    "),
                    ),
                    format!("with manager:\n    {}", guarded.replace('\n', "\n    ")),
                    guarded.clone(),
                    format!(
                        "if flag:\n    {}\nconsume(value)",
                        guarded.replace('\n', "\n    "),
                    ),
                ] {
                    let source = format!(
                        "{prefix}def function(value, items, manager, flag):\n    result = value\n{leading}    {}\n    return result\n",
                        body.replace('\n', "\n    "),
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "bare_handler_normal_exit_borrows.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_nested_normal_finally_empty_exits() {
    for prefix in ["", "async "] {
        for handler in ["", " ValueError", " ValueError as error"] {
            for body in [
                "if x:\n    action(y)",
                "if x:\n    action(y)\nelse:\n    action(x)",
                "if x:\n    if y:\n        action(y)\n    else:\n        action(x)",
            ] {
                let source = format!(
                    "{prefix}def f(x, y):\n    try:\n        action(y)\n    finally:\n        try:\n            {}\n        except{handler}:\n            pass\n    return consume(y)\n",
                    body.replace('\n', "\n            "),
                );
                common::assert_matches_cpython_marshal(&source, "nested_finally_exit.py");
            }
        }
    }
}

#[test]
fn preserves_cpython_nested_normal_finally_located_nops() {
    // These shapes retain independent exception-table differences. Their located NOPs
    // must survive the empty-block borrow boundary, including folded-away conditions.
    const ORACLE: &str = r#"
import dis, marshal, pathlib, sys, types
source = pathlib.Path(sys.argv[1]).read_text()
ours = marshal.loads(pathlib.Path(sys.argv[2]).read_bytes())
reference = compile(source, "nested_finally_nops.py", "exec", dont_inherit=True, optimize=0)
def nops(module):
    code = next(value for value in module.co_consts if isinstance(value, types.CodeType))
    return [tuple(instruction.positions) for instruction in dis.get_instructions(code)
            if instruction.opname == "NOP"]
expected = nops(reference)
assert expected, "oracle must retain located NOPs"
assert nops(ours) == expected, "nested-finally located NOPs differ"
"#;
    for prefix in ["", "async "] {
        for handler in ["", " ValueError"] {
            for body in ["if False:\n    action(y)", "if x:\n    pass"] {
                let source = format!(
                    "{prefix}def f(x, y):\n    try:\n        action(y)\n    finally:\n        try:\n            {}\n        except{handler}:\n            pass\n    return consume(y)\n",
                    body.replace('\n', "\n            "),
                );
                let directory = tempfile::tempdir().unwrap();
                let source_path = directory.path().join("source.py");
                let marshal_path = directory.path().join("ours.marshal");
                std::fs::write(&source_path, &source).unwrap();
                std::fs::write(
                    &marshal_path,
                    serc::compile(&source, "nested_finally_nops.py")
                        .unwrap()
                        .marshal(),
                )
                .unwrap();
                let output = common::python_3145_command()
                    .args(["-c", ORACLE])
                    .arg(&source_path)
                    .arg(&marshal_path)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}

#[test]
fn preserves_cpython_nested_finally_else_nop() {
    common::assert_matches_cpython_marshal(
        "def f(x, y):\n    try:\n        action(y)\n    finally:\n        try:\n            action(y)\n        except ValueError:\n            pass\n        else:\n            if False:\n                action(y)\n    return consume(y)\n",
        "nested_finally_else_nop.py",
    );
}

#[test]
fn matches_cpython_compound_finally_entries_inside_managers() {
    fn indent(source: &str) -> String {
        format!("    {}\n", source.trim_end().replace('\n', "\n    "))
    }

    for scope in 0..3 {
        let prefix = if scope == 1 { "async " } else { "" };
        let suspension = match scope {
            1 => "await ",
            2 => "yield ",
            _ => "",
        };
        for nested in [false, true] {
            for is_loop in [false, true] {
                let argument = if is_loop { "item" } else { "value" };
                let operation = format!("{suspension}action({argument})");
                let body = if is_loop {
                    format!("for item in items:\n    {operation}\n")
                } else {
                    format!("{operation}\n")
                };
                for handler in [
                    "except ValueError:\n    handle(value)\n",
                    "except ValueError as error:\n    handle(error)\n",
                ] {
                    for finalizer in [
                        "result = cleanup(value)\nconsume(result)\n",
                        "try:\n    cleanup(value)\nexcept OtherError:\n    pass\nconsume(value)\n",
                    ] {
                        let suite = format!(
                            "try:\n{}{handler}finally:\n{}",
                            indent(&body),
                            indent(finalizer),
                        );
                        let mut suite = format!("with manager:\n{}", indent(&suite));
                        if nested {
                            suite = format!("with outer:\n{}", indent(&suite));
                        }
                        suite.push_str("return None\n");
                        let source = format!(
                            "{prefix}def f(items, value, manager, outer):\n{}",
                            indent(&suite),
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "compound_finally_manager_entry.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_conditional_normal_finally_bypass_borrows() {
    for body in [
        "await action(value)\nif flag:\n    action(value)\n",
        "if flag:\n    await action(value)\n",
    ] {
        for handler in ["pass", "log(value)"] {
            let nested = format!(
                "try:\n{}except Exception:\n    {handler}\n",
                indent_finally_borrow_case(body)
            );
            let finalizer = format!("if flag:\n{}", indent_finally_borrow_case(&nested));
            let suite = format!(
                "try:\n    action(value)\nfinally:\n{}consume(value)\n",
                indent_finally_borrow_case(&finalizer)
            );
            for enclosing_loop in [None, Some("for item in items:"), Some("while flag:")] {
                let suite = match enclosing_loop {
                    None => suite.clone(),
                    Some(header) => format!(
                        "{header}\n{}{}consume(value)\n",
                        indent_finally_borrow_case(&suite),
                        if header.starts_with("while") {
                            "    break\n"
                        } else {
                            ""
                        }
                    ),
                };
                let source = format!(
                    "async def f(flag, value, items):\n{}",
                    indent_finally_borrow_case(&suite)
                );
                common::assert_matches_cpython_marshal(&source, "conditional_finally_bypass.py");
            }
        }
    }
}

#[test]
fn preserves_cpython_loops_entered_inside_normal_finally() {
    for prefix in ["", "async "] {
        for branch in [false, true] {
            let mut finalizer = "for item in items:\n    try:\n        if flag:\n            action(value)\n    except Exception:\n        pass\n".to_owned();
            if branch {
                finalizer = format!("if flag:\n{}", indent_finally_borrow_case(&finalizer));
            }
            let source = format!(
                "{prefix}def f(flag, value, items):\n    try:\n        action(value)\n    finally:\n{}    return consume(value)\n",
                indent_finally_borrow_case(&indent_finally_borrow_case(&finalizer))
            );
            common::assert_matches_cpython_marshal(&source, "loop_inside_finally_borrows.py");
        }
    }
}

#[test]
fn preserves_cpython_unconditional_nested_finally_borrows() {
    for scope in 0..3 {
        let prefix = if scope == 1 { "async " } else { "" };
        let action = if scope == 1 {
            "await action(value)"
        } else {
            "action(value)"
        };
        let entry = if scope == 2 { "    yield value\n" } else { "" };
        for header in ["for item in items:", "while flag:"] {
            for leading in ["", "            unused: object\n"] {
                let source = format!(
                    "{prefix}def f(flag, value, items):\n{entry}    {header}\n        try:\n            action(value)\n        finally:\n{leading}            try:\n                try:\n                    if flag:\n                        {action}\n                except Exception:\n                    pass\n            finally:\n                action(value)\n    consume(value)\n"
                );
                common::assert_matches_cpython_marshal(&source, "unconditional_nested_finally.py");
            }
        }
    }
}

#[test]
fn preserves_cpython_borrows_before_nested_pending_finalizers() {
    for scope in 0..3 {
        let prefix = if scope == 1 { "async " } else { "" };
        let action = if scope == 1 {
            "await action(value)"
        } else {
            "action(value)"
        };
        let entry = if scope == 2 { "    yield value\n" } else { "" };
        let inner = format!("try:\n    if flag:\n        {action}\nexcept Exception:\n    pass\n");
        let with_body = format!("with cm:\n{}", indent_finally_borrow_case(&inner));
        let with_finalizer = format!(
            "try:\n{}finally:\n    action(value)\n",
            indent_finally_borrow_case(&with_body)
        );
        let nested_finalizer = format!(
            "try:\n{}finally:\n    action(value)\n",
            indent_finally_borrow_case(&inner)
        );
        let branch = format!(
            "if guard:\n{}",
            indent_finally_borrow_case(&nested_finalizer)
        );
        for (finalizer, outer_loop) in [(with_finalizer, false), (branch, true)] {
            let suite = format!(
                "try:\n    action(value)\nfinally:\n{}",
                indent_finally_borrow_case(&finalizer)
            );
            let suite = if outer_loop {
                format!("for item in items:\n{}", indent_finally_borrow_case(&suite))
            } else {
                suite
            };
            let source = format!(
                "{prefix}def f(flag, guard, value, items, cm):\n{entry}{}    consume(value)\n",
                indent_finally_borrow_case(&suite)
            );
            common::assert_matches_cpython_marshal(&source, "nested_pending_finalizer.py");
        }
    }
}

#[test]
fn preserves_cpython_nested_handler_body_finally_borrows() {
    for scope in 0..3 {
        let prefix = if scope == 1 { "async " } else { "" };
        let action = if scope == 1 {
            "await action(value)"
        } else {
            "action(value)"
        };
        let entry = if scope == 2 { "    yield value\n" } else { "" };
        for leading in [false, true] {
            let body = format!(
                "{}if flag:\n    {action}\n",
                if leading {
                    format!("{action}\n")
                } else {
                    String::new()
                }
            );
            let inner = format!(
                "try:\n{}except Exception:\n    pass\n",
                indent_finally_borrow_case(&body)
            );
            let handler = format!(
                "try:\n{}except ValueError:\n    pass\n",
                indent_finally_borrow_case(&inner)
            );
            for depth in [0, 2] {
                let mut suite = format!(
                    "try:\n    action(value)\nfinally:\n{}consume(value)\n",
                    indent_finally_borrow_case(&handler)
                );
                for _ in 0..depth {
                    suite = format!(
                        "try:\n{}finally:\n    cleanup(value)\n",
                        indent_finally_borrow_case(&suite)
                    );
                }
                let source = format!(
                    "{prefix}def f(flag, value):\n{entry}{}",
                    indent_finally_borrow_case(&suite)
                );
                common::assert_matches_cpython_marshal(&source, "nested_handler_body_finally.py");
            }
        }
    }
}

#[test]
fn matches_cpython_surviving_nested_finally_handler_jumps() {
    for scope in 0..3 {
        let prefix = if scope == 1 { "async " } else { "" };
        let action = if scope == 1 {
            "await action(value)"
        } else {
            "action(value)"
        };
        let entry = if scope == 2 { "    yield value\n" } else { "" };
        for leading in [false, true] {
            let body = format!(
                "{}if flag:\n    {action}\n",
                if leading {
                    format!("{action}\n")
                } else {
                    String::new()
                }
            );
            let inner = format!(
                "try:\n{}except Exception:\n    pass\n",
                indent_finally_borrow_case(&body)
            );
            let conditional = format!("if guard:\n{}", indent_finally_borrow_case(&inner));
            let handler = format!(
                "try:\n{}except ValueError:\n    pass\n",
                indent_finally_borrow_case(&conditional)
            );
            for depth in 0..3 {
                let mut suite = format!(
                    "try:\n    action(value)\nfinally:\n{}consume(value)\n",
                    indent_finally_borrow_case(&handler)
                );
                for _ in 0..depth {
                    suite = format!(
                        "try:\n{}finally:\n    cleanup(value)\n",
                        indent_finally_borrow_case(&suite)
                    );
                }
                let source = format!(
                    "{prefix}def f(flag, guard, value):\n{entry}{}",
                    indent_finally_borrow_case(&suite)
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "surviving_finally_handler_jump.py",
                );
            }
        }
    }
}

#[test]
fn preserves_borrows_after_removed_nested_finally_jumps() {
    let function = r"def f(flag, guard, value, cm):
    try:
        action(value)
    finally:
        if guard:
            if guard:
                try:
                    if flag:
                        action(value)
                except Exception:
                    pass
    consume(value)
";
    // This shape has a separate jump-layout mismatch. Compare every local-load opcode
    // against a fresh native compilation so that partial borrowing regressions still fail.
    let source = format!(
        r#"{function}
import dis
native = compile({function:?}, "native_finally_jump.py", "exec", dont_inherit=True, optimize=0).co_consts[0]
def loads(code):
    return [(instruction.opname, instruction.argval) for instruction in dis.get_instructions(code) if instruction.opname.startswith("LOAD_FAST")]
expected = loads(native)
actual = loads(f.__code__)
assert any(opname == "LOAD_FAST" for opname, _ in expected), expected
assert actual == expected, (actual, expected)
print("matched")
"#
    );
    common::assert_executes(&source, "matched\n");
}

fn indent_finally_borrow_case(source: &str) -> String {
    let mut result = String::new();
    for line in source.lines() {
        result.push_str("    ");
        result.push_str(line);
        result.push('\n');
    }
    result
}

#[test]
fn matches_cpython_name_returns_after_handlers_with_manager_cleanup() {
    fn indent(source: &str) -> String {
        format!("    {}\n", source.trim_end().replace('\n', "\n    "))
    }

    for (prefix, manager) in [
        ("", "with manager"),
        ("", "with manager()"),
        ("async ", "async with manager"),
    ] {
        for handler in ["pass", "raise", "return None"] {
            let body = format!(
                "{manager}:\n    try:\n        action(value)\n    except Error:\n        {handler}\n    return value\n"
            );
            for in_loop in [false, true] {
                let body = if in_loop {
                    format!("for value in values:\n{}", indent(&body))
                } else {
                    body.clone()
                };
                let source = format!("{prefix}def f(value, manager, values):\n{}", indent(&body));
                common::assert_matches_cpython_marshal(&source, "with_name_return.py");
            }
        }
    }
    common::assert_matches_cpython_marshal(
        "def f(value, manager, values):\n    for value in values:\n        with manager:\n            try:\n                action(value)\n            except Error:\n                continue\n            return value\n",
        "with_name_return_continue.py",
    );
}

#[test]
fn matches_cpython_mixed_boolean_try_exit_borrows() {
    for test in [
        "flag or (other and value)",
        "flag or other or (value is not None and check(value))",
    ] {
        for action in ["raise Error", "return None"] {
            for handler in ["raise Error", "return None", "pass", "value = other"] {
                let source = format!(
                    "def f(value, flag, other):\n    try:\n        if {test}:\n            {action}\n    except Exception:\n        {handler}\n    consume(value)\n    return value\n"
                );
                common::assert_matches_cpython_marshal(&source, "mixed_boolean_try_exit.py");
            }
        }
    }
}

#[test]
fn preserves_cpython_mixed_boolean_try_exit_copy_limits_and_shared_joins() {
    for test in ["flag or (other and value)", "flag and other"] {
        for tail in [
            "return value",
            "return touch(value)",
            "return value + 1",
            "other = value\nreturn value",
            "other = value + 1\nreturn value",
            "consume(value)\nreturn value",
            "return None",
            "value\nreturn value",
        ] {
            for shared_join in [false, true] {
                let protected = format!(
                    "try:\n    if {test}:\n        raise Error\nexcept Exception:\n    raise Error\n"
                );
                let protected = if shared_join {
                    format!(
                        "if guard:\n{}else:\n    consume(other)\n",
                        indent_finally_borrow_case(&protected)
                    )
                } else {
                    protected
                };
                let source = format!(
                    "def f(value, flag, other, guard):\n{}{}",
                    indent_finally_borrow_case(&protected),
                    indent_finally_borrow_case(tail)
                );
                common::assert_matches_cpython_marshal(&source, "mixed_boolean_try_join.py");
            }
        }
    }
}

#[test]
fn matches_cpython_empty_nested_try_exits_after_terminal_conditions() {
    for (prefix, leading) in [("", ""), ("async ", ""), ("", "    yield value\n")] {
        for terminal in ["raise Error", "return None"] {
            for nested in [
                "try:\n    work()\nexcept Error:\n    recover()",
                "try:\n    work()\nfinally:\n    recover()",
            ] {
                for handler in [
                    "except Error:\n    pass",
                    "except Error as error:\n    raise",
                ] {
                    let source = format!(
                        "{prefix}def function(value, flag):\n{leading}    try:\n        if flag:\n            {terminal}\n        {}\n    {}\n    consume(value)\n    return value\n",
                        nested.replace('\n', "\n        "),
                        handler.replace('\n', "\n    "),
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "terminal_condition_nested_try_exit.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_try_exit_after_multi_edge_while_condition() {
    for condition in [
        "xs",
        "xs or value",
        "xs and value",
        "xs and (value or flag)",
        "not (xs or value)",
    ] {
        for handler in [
            "except Error:\n        return None",
            "except Error as exc:\n        raise OtherError() from exc",
        ] {
            let source = format!(
                "def f(xs, value, flag):\n    try:\n        while {condition}:\n            work(xs)\n            xs = next_value(xs)\n    {handler}\n    return consume(value)\n"
            );
            common::assert_matches_cpython_marshal(&source, "try_while_condition_exit.py");
        }
    }
}

fn browser_finally_borrow_source(async_prefix: &str, cleanup: &str, handler: &str) -> String {
    let await_prefix = if async_prefix.is_empty() {
        ""
    } else {
        "await "
    };
    let cleanup = cleanup.replace('\n', "\n            ");
    format!(
        "{async_prefix}def f(value, flag, other, manager):\n    try:\n        if other:\n            raise Error\n        try:\n            result = {await_prefix}action()\n        finally:\n            {cleanup}\n    {handler}\n    consume(value)\n    return result, value\n"
    )
}

#[test]
fn matches_cpython_browser_finally_borrow_conditional_cleanups() {
    // A terminal branch before the nested try must not make structured cleanup
    // look like a plain call when deciding whether the final load can borrow.
    for (kind, async_prefix, await_prefix) in [("sync", "", ""), ("async", "async ", "await ")] {
        for (cleanup_name, cleanup) in [
            (
                "conditional_with",
                "if flag:\n    with manager:\n        cleanup()",
            ),
            (
                "conditional_try",
                "if flag:\n    try:\n        cleanup()\n    except Error:\n        pass",
            ),
            (
                "conditional_ifelse",
                "if flag:\n    if other:\n        cleanup()\n    else:\n        cleanup()",
            ),
            (
                "if_else_with",
                "if flag:\n    with manager:\n        cleanup()\nelse:\n    cleanup()",
            ),
            (
                "if_else_try",
                "if flag:\n    try:\n        cleanup()\n    except Error:\n        pass\nelse:\n    cleanup()",
            ),
            (
                "if_else_calls",
                "if flag:\n    cleanup()\nelse:\n    cleanup()",
            ),
            (
                "conditional_for_break",
                "if flag:\n    for item in items:\n        if other:\n            break\n        cleanup()",
            ),
        ] {
            let cleanup = cleanup.replace("cleanup()", &format!("{await_prefix}cleanup()"));
            for (handler_name, handler) in [
                ("bare", "except Error:\n        raise"),
                (
                    "named",
                    "except Error as exc:\n        raise Other from exc",
                ),
            ] {
                let source = browser_finally_borrow_source(async_prefix, &cleanup, handler);
                common::assert_matches_cpython_marshal(
                    &source,
                    &format!("browser_finally_borrow_{cleanup_name}_{kind}_{handler_name}.py"),
                );
            }
        }
    }
}

#[test]
fn matches_cpython_browser_finally_borrow_plain_cleanup() {
    for (kind, async_prefix, cleanup) in [
        ("sync", "", "cleanup()"),
        ("async", "async ", "await cleanup()"),
    ] {
        for (handler_name, handler) in [
            ("bare", "except Error:\n        raise"),
            (
                "named",
                "except Error as exc:\n        raise Other from exc",
            ),
        ] {
            let source = browser_finally_borrow_source(async_prefix, cleanup, handler);
            common::assert_matches_cpython_marshal(
                &source,
                &format!("browser_finally_borrow_call_{kind}_{handler_name}.py"),
            );
        }
    }
}

#[test]
fn matches_cpython_browser_finally_borrow_neighboring_cleanups() {
    // These async controls await the action but leave the cleanup synchronous.
    for (kind, async_prefix) in [("sync", ""), ("async", "async ")] {
        for (cleanup_name, cleanup) in [
            ("try", "try:\n    cleanup()\nexcept Other:\n    cleanup()"),
            ("if_else", "if flag:\n    cleanup()\nelse:\n    cleanup()"),
            (
                "conditional_with_then_call",
                "if flag:\n    with manager:\n        cleanup()\ncleanup()",
            ),
        ] {
            for (handler_name, handler) in [
                ("bare", "except Error:\n        raise"),
                ("named", "except Error as error:\n        raise"),
            ] {
                let source = browser_finally_borrow_source(async_prefix, cleanup, handler);
                common::assert_matches_cpython_marshal(
                    &source,
                    &format!("browser_finally_borrow_{cleanup_name}_{kind}_{handler_name}.py"),
                );
            }
        }
    }
}

#[test]
fn matches_cpython_loop_tail_nested_try_borrow_reachability() {
    // Explicit loop tails and ordinary forward successors visit different empty
    // blocks during CPython borrowing analysis. Keep the earlier helper cases
    // covered with complete marshal comparisons as the nested-try case evolves.
    for (tail_name, tail, contexts) in [
        (
            "nested_no_else",
            "try:\n    work(value)\nexcept Other:\n    pass",
            &["handler_tail_if", "straight_handler", "forward_successor"][..],
        ),
        (
            "named_ifexp_assignment",
            "try:\n    result = value if check() else other\nexcept Other as error:\n    result = other",
            &["straight_handler", "forward_successor"][..],
        ),
        (
            "retained_else_assert",
            "try:\n    work(value)\nexcept Other:\n    work(value)\nelse:\n    assert flag",
            &["straight_handler", "forward_successor"][..],
        ),
        (
            "retained_else_terminal_if",
            "try:\n    work(value)\nexcept Other:\n    work(value)\nelse:\n    if flag:\n        return value",
            &["straight_handler", "forward_successor"][..],
        ),
        (
            "nested_populated_else",
            "try:\n    work(value)\nexcept Other:\n    work(value)\nelse:\n    cleanup(value)",
            &["handler_tail_if", "straight_handler"][..],
        ),
        (
            "fallthrough_if",
            "if other:\n    work(value)",
            &["handler_tail_if", "straight_handler", "forward_successor"][..],
        ),
        (
            "fallthrough_if_with",
            "if other:\n    with manager:\n        work(value)",
            &["handler_tail_if", "straight_handler", "forward_successor"][..],
        ),
        (
            "multi_exit_while",
            "while other and flag:\n    work(value)",
            &["forward_successor"][..],
        ),
    ] {
        for context in contexts {
            let (handler, following) = match *context {
                "handler_tail_if" => ("if other:\n    work(value)", ""),
                "straight_handler" => ("work(value)", ""),
                "forward_successor" => ("if other:\n    work(value)", "        consume(value)\n"),
                _ => unreachable!(),
            };
            let tail = tail.replace('\n', "\n            ");
            let handler = handler.replace('\n', "\n            ");
            let source = format!(
                "def f(values, value, flag, other, manager):\n    for item in values:\n        try:\n            if flag:\n                continue\n            {tail}\n        except Error:\n            {handler}\n{following}    return value\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                &format!("qstar_loop_tail_{tail_name}_{context}.py"),
            );
        }
    }
}

#[test]
fn matches_cpython_nonempty_normal_finally_entries() {
    for prefix in ["", "async "] {
        for handler in ["pass", "raise"] {
            for cleanup in [
                "for item in values:\n    consume(item)",
                "with manager:\n    consume(x)",
                "try:\n    consume(x)\nexcept Exception:\n    pass",
                "consume(x)\nfor item in values:\n    consume(item)",
                "try:\n    while flag:\n        consume(x)\nexcept Exception:\n    pass",
            ] {
                let source = format!(
                    "{prefix}def f(x, y, values, manager, flag):\n    try:\n        if flag:\n            work(x)\n        else:\n            work(y)\n    except Exception:\n        {handler}\n    finally:\n        {}\n",
                    cleanup.replace('\n', "\n        "),
                );
                common::assert_matches_cpython_marshal(&source, "nonempty_finally_entry.py");
            }
        }
    }
}

#[test]
fn preserves_cpython_plain_conditional_finally_entries() {
    for cleanup in [
        "for item in values:\n    consume(item)",
        "with manager:\n    consume(x)",
        "try:\n    consume(x)\nexcept Exception:\n    pass",
    ] {
        let source = format!(
            "def f(x, y, values, manager, flag):\n    for outer in values:\n        try:\n            if flag:\n                work(x)\n        except Exception:\n            continue\n        finally:\n            {}\n",
            cleanup.replace('\n', "\n            "),
        );
        common::assert_matches_cpython_marshal(&source, "plain_conditional_finally_entry.py");
    }
    common::assert_matches_cpython_marshal(
        "def f(x, flag):\n    try:\n        if flag:\n            work(x)\n    except Exception:\n        pass\n    finally:\n        while flag:\n            consume(x)\n",
        "leading_while_finally_entry.py",
    );
}

#[test]
fn matches_cpython_nested_finally_loop_successor_boundary() {
    common::assert_matches_cpython_marshal(
        "def f(flag, x, y, values):\n    try:\n        if flag:\n            consume(x)\n        else:\n            consume(y)\n    except Exception:\n        pass\n    finally:\n        try:\n            consume(x)\n        except Exception:\n            pass\n        if flag:\n            for item in values:\n                if flag:\n                    consume(item)\n        consume(x)\n    consume(x, y)\n",
        "nested_finally_loop_successor.py",
    );
}

#[test]
fn matches_cpython_folded_finally_conditional_successor_boundary() {
    for body in [
        "try:\n    consume(x)\nexcept Exception:\n    pass",
        "with manager:\n    consume(x)",
    ] {
        for suffix in ["", "        consume(x)\n"] {
            let source = format!(
                "def f(flag, x, y, manager):\n    try:\n        if flag:\n            consume(x)\n        else:\n            consume(y)\n    except Exception:\n        pass\n    finally:\n        consume(x)\n        if True:\n            {}\n{suffix}    consume(x, y)\n",
                body.replace('\n', "\n            "),
            );
            common::assert_matches_cpython_marshal(&source, "folded_finally_successor.py");
        }
    }
}

#[test]
fn preserves_nested_try_body_conditional_borrow_successors() {
    for terminal in ["raise ValueError", "return value"] {
        for body in [
            "if flag:\n    use(value)",
            "if flag:\n    use(value)\nelse:\n    other(value)",
            "if flag:\n    use(value)\nelif other_flag:\n    other(value)\nelse:\n    finish(value)",
            "if flag:\n    use(value)\nelse:\n    return value",
            "try:\n    use(value)\nexcept OtherError:\n    other(value)",
            "with manager:\n    use(value)",
            "if flag:\n    try:\n        use(value)\n    except OtherError:\n        other(value)",
            "if flag:\n    with manager:\n        use(value)",
        ] {
            for cleanup in [
                "except Exception:\n    recover(value)",
                "finally:\n    close(value)",
                "finally:\n    if flag:\n        close(value)",
                "finally:\n    if flag:\n        close(value)\n    else:\n        finish(value)",
            ] {
                let source = format!(
                    "def f(value, flag, other_flag, manager):\n    try:\n        if other_flag:\n            {terminal}\n        else:\n            work(value)\n        try:\n            {}\n        {}\n    except Exception:\n        raise\n    if value is not None:\n        use(value)\n    return value\n",
                    body.replace('\n', "\n            "),
                    cleanup.replace('\n', "\n        "),
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "nested_try_body_conditional_borrow_successors.py",
                );
            }
        }
    }
}

#[test]
fn retained_try_exit_nops_preserve_borrowed_load_reachability() {
    for (name, source) in [
        (
            "otter_loop",
            r"def f(manager, flag, x):
    while flag:
        with manager:
            try:
                result = check()
                if result:
                    return True
                else:
                    work()
            except Error:
                recover()
        consume(x)
    return False
",
        ),
        (
            "no_loop",
            r"def f(manager, flag, x):
    with manager:
        try:
            result = check()
            if result:
                return True
            else:
                work()
        except Error:
            recover()
    consume(x)
",
        ),
        (
            "straight_try",
            r"def f(manager, flag, x):
    with manager:
        try:
            work()
        except Error:
            recover()
    consume(x)
",
        ),
        (
            "no_else",
            r"def f(manager, flag, x):
    with manager:
        try:
            result = check()
            if result:
                return True
        except Error:
            recover()
    consume(x)
",
        ),
        (
            "tail_try_finally_for",
            r"def f(manager, items, x):
    with manager:
        try:
            for item in items:
                work(item)
        finally:
            cleanup()
    return x
",
        ),
        (
            "no_with",
            r"def f(manager, flag, x):
    try:
        result = check()
        if result:
            return True
        else:
            work()
    except Error:
        recover()
    consume(x)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, &format!("retained_try_exit_{name}.py"));
    }
}

#[test]
fn async_loop_break_joins_preserve_manager_borrow_barriers() {
    for (name, source) in [
        (
            "asyncfor_break_call",
            r"async def f(manager, items, flag, x):
    with manager:
        try:
            async for item in items:
                work(item)
                if flag:
                    break
        finally:
            cleanup()
    return x
",
        ),
        (
            "asyncfor_break_await",
            r"async def f(manager, items, flag, x):
    with manager:
        try:
            async for item in items:
                work(item)
                if flag:
                    break
        finally:
            cleanup()
            await finish()
    return x
",
        ),
    ] {
        common::assert_matches_cpython_marshal(
            source,
            &format!("async_loop_borrow_barrier_{name}.py"),
        );
    }
}

#[test]
fn matches_cpython_owned_continuation_after_conditional_recovery_orphan() {
    // The located handler suffix prevents shared-cleanup copying, isolating the
    // original jump-only block left by the nested try in the nonfinal arm.
    let source = r#"def f(flag, token_id, cache, decode, decode_bytes):
    try:
        decoded = decode(token_id)
    except Exception:
        if flag:
            try:
                decoded = decode_bytes(token_id)
            except Exception:
                decoded = f"<tok:{token_id}>"
        else:
            decoded = f"<tok:{token_id}>"
        work()
    cache[token_id] = decoded
    return decoded
"#;
    common::assert_matches_cpython_marshal(source, "conditional_recovery_orphan.py");
}

#[test]
fn matches_cpython_owned_continuation_after_final_else_recovery_cleanup() {
    let source = r"def f(flag, token_id, cache, decode, decode_bytes):
    decoded = 0
    try:
        decoded = decode(token_id)
    except Exception as outer_error:
        if flag:
            decoded = token_id
        else:
            try:
                decoded = decode_bytes(token_id)
            except Exception as inner_error:
                decoded = token_id
    cache[token_id] = decoded
    return decoded
";
    common::assert_matches_cpython_marshal(source, "final_else_recovery_cleanup.py");
}

#[test]
fn matches_cpython_owned_try_else_continuation_after_conditional_recovery() {
    let source = r"def f(flag, token_id, cache, decode, decode_bytes):
    decoded = 0
    try:
        decoded = decode(token_id)
    except Exception:
        if flag:
            try:
                decoded = decode_bytes(token_id)
            except Exception:
                decoded = token_id
        else:
            decoded = token_id
        work()
    else:
        decoded = None
    cache[token_id] = decoded
    return decoded
";
    common::assert_matches_cpython_marshal(source, "try_else_conditional_recovery.py");
}

#[test]
fn preserves_native_borrowed_prefix_after_noelse_recovery() {
    use std::io::Write as _;
    use std::process::Stdio;

    // These sources still differ in later shared-cleanup layout. Their normal
    // prefix was already native-exact; preserve its bytes and source positions.
    // Keep the whole-marshal checks above and the corpus checks unchanged.
    let oracle = r#"
import dis, marshal, sys
actual = marshal.loads(sys.stdin.buffer.read()).co_consts[0]
expected = compile(sys.argv[1], sys.argv[2], "exec", dont_inherit=True, optimize=0).co_consts[0]
assert expected.co_code[24] == dis.opmap["LOAD_FAST_BORROW_LOAD_FAST_BORROW"]
assert expected.co_code[26] == expected.co_code[32] == dis.opmap["LOAD_FAST_BORROW"]
assert actual.co_code[:36] == expected.co_code[:36]
assert list(actual.co_positions())[:18] == list(expected.co_positions())[:18]
assert actual.co_varnames == expected.co_varnames
"#;
    for named in [false, true] {
        let outer = if named {
            "except Exception as outer_error:"
        } else {
            "except Exception:"
        };
        let inner = if named {
            "except Exception as inner_error:"
        } else {
            "except Exception:"
        };
        let source = format!(
            "def f(flag, other, token_id, cache, decode, decode_bytes):\n    decoded = 0\n    try:\n        decoded = decode(token_id)\n    {outer}\n        if flag:\n            try:\n                decoded = decode_bytes(token_id)\n            {inner}\n                decoded = token_id\n    cache[token_id] = decoded\n    return decoded\n",
        );
        let filename = "noelse_recovery_borrow_prefix.py";
        let actual = serc::compile(&source, filename).unwrap().marshal();
        let mut child = common::python_3145_command()
            .args(["-c", oracle, &source, filename])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&actual).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "native borrowed prefix changed for named={named}: {}",
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

#[test]
fn matches_cpython_continuation_after_loop_recovery_orphans() {
    for source in [
        r"def f(items):
    x = None
    try:
        x = work()
    except OuterError:
        for item in items:
            try:
                x = parse(item)
            except InnerError:
                x = None
        if x is None:
            return None
    return consume(x)
",
        r"def f(items):
    x = None
    try:
        x = work()
    except OuterError:
        for item in items:
            try:
                x = parse(item)
                break
            except InnerError:
                x = None
        if x is None:
            return None
    return consume(x)
",
        r"def f(flag):
    x = None
    try:
        x = work()
    except OuterError:
        while condition(flag):
            try:
                x = work_again()
            except InnerError as inner_error:
                x = inner_error
        x = finish()
    return consume(x)
",
        r"async def f(task):
    result = None
    cancelled = None
    try:
        result = await shield(task)
    except OuterError as exc:
        cancelled = exc
        while not task.done():
            try:
                await wait(task)
            except InnerError as repeat:
                cancelled = repeat
        result = task.result()
    if cancelled is not None:
        notify(cancelled)
    return consume(result)
",
        r"def f(items):
    x = None
    try:
        x = work()
    except OuterError:
        for item in items:
            try:
                x = parse(item)
            except InnerError:
                x = None
    return consume(x)
",
    ] {
        common::assert_matches_cpython_marshal(source, "loop_recovery_orphan.py");
    }
}

#[test]
fn preserves_cpython_borrowing_across_other_loop_exits() {
    for source in [
        r"def f(items):
    x = None
    try:
        x = work()
    except OuterError:
        for item in items:
            x = parse(item)
        if x is None:
            return None
    return consume(x)
",
        r"def f(flag):
    x = None
    try:
        x = work()
    except OuterError:
        while condition(flag):
            x = work_again()
        x = finish()
    return consume(x)
",
        r"def f(items, flag):
    x = None
    item = None
    try:
        x = work()
    except OuterError:
        while condition(flag):
            x = parse(item)
            break
        if x is None:
            return None
    return consume(x)
",
        r"def f(items, flag):
    x = None
    item = None
    try:
        x = work()
    except OuterError:
        while True:
            try:
                x = parse(item)
            except InnerError:
                x = None
        if x is None:
            return None
    return consume(x)
",
        r"def f(flag):
    x = None
    try:
        x = work()
    except OuterError:
        while not True:
            try:
                x = work_again()
            except InnerError:
                x = None
        x = finish()
    return consume(x)
",
    ] {
        common::assert_matches_cpython_marshal(source, "loop_recovery_orphan.py");
    }
}

#[test]
fn preserves_native_borrowed_prefix_after_folded_false_recovery_loop() {
    use std::io::Write as _;
    use std::process::Stdio;

    // This source retains an inherited marshal mismatch. Its normal prefix was
    // native-exact before orphan detection; preserve its bytes and positions.
    let oracle = r#"
import dis, marshal, sys
actual = marshal.loads(sys.stdin.buffer.read()).co_consts[0]
expected = compile(sys.argv[1], sys.argv[2], "exec", dont_inherit=True, optimize=0).co_consts[0]
assert expected.co_code[38] == dis.opmap["LOAD_FAST_BORROW"]
assert actual.co_code[:40] == expected.co_code[:40]
assert list(actual.co_positions())[:20] == list(expected.co_positions())[:20]
assert actual.co_varnames == expected.co_varnames
"#;
    let source = r"def f(flag):
    x = None
    try:
        x = work()
    except OuterError:
        while (0 or False):
            try:
                x = work_again()
            except InnerError:
                x = None
        x = finish()
    return consume(x)
";
    let filename = "folded_false_recovery_borrow_prefix.py";
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
        "native borrowed prefix changed after a folded false loop: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}
