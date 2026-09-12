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

#[test]
fn linear_for_finalizer_reaches_manager_continuation() {
    let source = r"def f(manager, items, value, work, observe):
    with manager:
        try:
            for item in items:
                work(item)
        finally:
            for item in items:
                item.cancel()
    observe(value)
    observe(value)
    return value
";
    common::assert_matches_cpython_marshal(source, "linear_for_finalizer_continuation.py");
}

#[test]
fn assigning_for_finalizer_reaches_manager_continuation() {
    let source = r"def f(manager, items, total, work, observe):
    with manager:
        try:
            work(total)
        finally:
            for item in items:
                total += item
                observe(total)
    observe(total)
    observe(total)
    return total
";
    common::assert_matches_cpython_marshal(source, "assigning_for_finalizer_continuation.py");
}

#[test]
fn conditional_for_finalizer_keeps_empty_backedge_barrier() {
    let source = r"def f(manager, items, value, flag, work, observe):
    with manager:
        try:
            work(value)
        finally:
            for item in items:
                if flag:
                    item.cancel()
    observe(value)
    observe(value)
    return value
";
    common::assert_matches_cpython_marshal(source, "conditional_for_finalizer_barrier.py");
}

#[test]
fn nested_try_for_finalizer_keeps_empty_backedge_barrier() {
    let source = r"def f(manager, items, value, work, observe):
    with manager:
        try:
            work(value)
        finally:
            for item in items:
                try:
                    item.cancel()
                except Exception:
                    pass
    observe(value)
    observe(value)
    return value
";
    common::assert_matches_cpython_marshal(source, "nested_try_for_finalizer_barrier.py");
}

#[test]
fn for_finalizer_cannot_reopen_empty_protected_loop_exit() {
    let source = r"def f(manager, items, value, observe):
    with manager:
        try:
            while True:
                break
        finally:
            for item in items:
                item.cancel()
    observe(value)
    observe(value)
    return value
";
    common::assert_matches_cpython_marshal(source, "for_finalizer_empty_protected_exit.py");
}

#[test]
fn matches_cpython_borrows_after_nested_terminal_try_else_arms() {
    for (name, body) in [
        ("return", "return parse()"),
        ("raise", "raise Inner"),
        ("assignment", "error = parse()"),
        ("conditional_return", "if other:\n    return parse()"),
    ] {
        let source = format!(
            "def f(flag, value, other):\n    error = None\n    try:\n        work()\n    except Outer:\n        error = 'outer'\n    else:\n        if flag:\n            try:\n                {}\n            except Inner as exc:\n                error = str(exc)\n        else:\n            error = fail()\n    if value:\n        emit(value)\n    return error\n",
            body.replace('\n', "\n                "),
        );
        common::assert_matches_cpython_marshal(
            &source,
            &format!("nested_terminal_try_else_{name}.py"),
        );
    }
}

#[test]
fn matches_cpython_borrows_in_retry_tail_after_nested_terminal_try() {
    common::assert_matches_cpython_marshal(
        r#"def f(output, limit, timeout):
    error = None
    for attempt in range(limit):
        try:
            result = runner.run(timeout=timeout)
        except runner.Timeout:
            error = "timed out"
        else:
            if result.status == 0 and output.exists():
                try:
                    return parser.loads(output.read_text().strip())
                except parser.ParseError as exc:
                    error = f"bad output: {exc}"
            else:
                message = (result.stderr or "").strip()
                error = message or f"failed: {result.status}"
        if attempt + 1 < limit:
            delay = 1.5 * (2**attempt)
            emit(f"retry {attempt + 2}/{limit} in {delay:.1f}s")
            clock.sleep(delay)
    raise Failure(error or "failed")
"#,
        "retry_tail_after_nested_terminal_try.py",
    );
}

#[test]
fn preserves_nested_terminal_try_borrow_and_location_boundaries() {
    for (name, source) in [
        (
            "both_nested_return",
            r#"def f(flag, value):
    error = None
    try:
        work()
    except Outer:
        error = "outer"
    else:
        if flag:
            try:
                return parse()
            except Inner as exc:
                error = str(exc)
        else:
            try:
                return fail()
            except Other:
                error = "other"
    if value:
        emit(value)
    return error
"#,
        ),
        (
            "location_pass_all_terminal",
            r"def f(flag, value):
    error = None
    try:
        work()
    except Outer:
        pass
    else:
        if flag:
            try:
                return parse()
            except Inner as exc:
                return str(exc)
    if value:
        emit(value)
    return error
",
        ),
    ] {
        common::assert_matches_cpython_marshal(
            source,
            &format!("nested_terminal_boundary_{name}.py"),
        );
    }
}

#[test]
fn nested_reraise_finally() {
    common::assert_matches_cpython_marshal(
        r"def f(value, emit):
    saved = value
    try:
        result = emit()
        try:
            result.wait()
            if result.failed:
                raise Failure()
        except Interrupt:
            result.stop()
            result.wait()
            raise
    finally:
        emit(saved)
",
        "nested_reraise_finally.py",
    );
}

#[test]
fn loop_break_named_handler_tail() {
    common::assert_matches_cpython_marshal(
        r"def f(items, emit):
    try:
        while True:
            result = emit(items)
            if result.pending:
                continue
            if result.failed:
                return 2
            rows = result.rows or []
            break
    except Exception as exc:
        emit(exc)
        return 2
    emit(len(rows))
    return 0
",
        "loop_break_named_handler_tail.py",
    );
}

#[test]
fn loop_break_named_handler_tail_two_calls() {
    common::assert_matches_cpython_marshal(
        r#"def f(items, emit):
    try:
        while True:
            result = emit(items)
            if result.pending:
                continue
            if result.failed:
                return 2
            rows = result.rows or []
            break
    except Exception as exc:
        emit(exc)
        return 2
    emit("done")
    emit(len(rows))
    return 0
"#,
        "loop_break_named_handler_tail_two_calls.py",
    );
}

#[test]
fn matches_cpython_manager_finally_borrow_route_control() {
    common::assert_matches_cpython_marshal(
        r#"def f(manager, lock, state, emit, mode):
    error = None
    with manager:
        with lock:
            if state.closed:
                raise Missing()
        state.updated = now()
        try:
            if mode == "read":
                result = emit(state)
            elif mode == "write":
                result = emit()
                if result:
                    with lock:
                        state.done = True
        except FirstError as exc:
            emit(str(exc))
            raise
        except Exception as exc:
            if getattr(exc, "special", False) is True:
                raise FirstError(str(exc)) from exc
            emit(str(exc))
            raise
        finally:
            if state.close_requested:
                try:
                    state.close()
                except Exception as exc:
                    error = str(exc) or type(exc).__name__
            state.updated = now()
    emit(result)
    if error is not None:
        emit(error)
    result.update(state.value)
    return result
"#,
        "manager_finally_borrow_route_control.py",
    );
}

#[test]
fn conditional_raise_before_break_finally() {
    common::assert_matches_cpython_marshal(
        r"def f(left, right):
    try:
        while True:
            a = left.poll()
            b = right.poll()
            if a is not None or b is not None:
                if a not in (None, 0):
                    code_a: int = a if a is not None else -1
                    raise Problem(code_a)
                if b not in (None, 0):
                    code_b: int = b if b is not None else -1
                    raise Problem(code_b)
                break
            tick()
    except Interrupt:
        pass
    finally:
        close(left)
        close(right)
",
        "conditional_raise_before_break_finally.py",
    );
}

#[test]
fn single_raise_before_break_finally() {
    common::assert_matches_cpython_marshal(
        r"def f(item):
    try:
        while True:
            value = item.poll()
            if value is not None:
                if value:
                    raise Problem(value)
                break
            tick()
    except Interrupt:
        pass
    finally:
        close(item)
        finish(item)
",
        "single_raise_before_break_finally.py",
    );
}

#[test]
fn simple_conditional_break_finally() {
    common::assert_matches_cpython_marshal(
        r"def f(item):
    try:
        while True:
            value = item.poll()
            if value is not None:
                break
            tick()
    except Interrupt:
        pass
    finally:
        close(item)
        finish(item)
",
        "simple_conditional_break_finally.py",
    );
}

#[test]
fn borrows_runtime_ifexp_branch_call_arguments() {
    let source = "def f(selector, first, second):\n    if (left(first, second) if selector else right(first, second)):\n        return make(status='ok', first=first, second=second, flag=selector)\n    after()\n";
    common::assert_matches_cpython_marshal(source, "runtime_ifexp_branch_call.py");
}

#[test]
fn borrows_runtime_ifexp_branch_inside_while() {
    let source = "def f(selector, first, second):\n    while running():\n        second = update()\n        if (left(first, second) if selector else right(first, second)):\n            return make(status='ok', first=first, second=second, flag=selector)\n        after()\n";
    common::assert_matches_cpython_marshal(source, "runtime_ifexp_loop_branch.py");
}

#[test]
fn borrows_after_protected_terminal_elif_breaks() {
    let source = "def f(item):\n    try:\n        while True:\n            state = read()\n            if state < 0:\n                use(item)\n            elif state == 0:\n                break\n            elif state == 1:\n                pause()\n                break\n        use(item)\n        use(item)\n        return item\n    except Error as exc:\n        report(exc)\n        return None\n    finally:\n        cleanup(item)\n";
    common::assert_matches_cpython_marshal(source, "protected_terminal_elif_break.py");
}

#[test]
fn borrows_after_unprotected_terminal_elif_breaks() {
    let source = "def f(item):\n    while True:\n        state = read()\n        if state < 0:\n            use(item)\n        elif state == 0:\n            break\n        elif state == 1:\n            pause()\n            break\n    use(item)\n    use(item)\n    return item\n";
    common::assert_matches_cpython_marshal(source, "unprotected_terminal_elif_break.py");
}

#[test]
fn borrows_chained_closure_cell_assignments() {
    for source in [
        "def f(value):\n    left = right = value\n    return lambda: (left, right)\n",
        "def f(value):\n    left = middle = right = value\n    return lambda: (left, middle, right)\n",
        "def f(value, flag):\n    if flag:\n        left = right = value\n    else:\n        left, right = value\n    return lambda: (left, right)\n",
    ] {
        common::assert_matches_cpython_marshal(source, "chained_closure_cell_assignments.py");
    }
}

#[test]
fn preserves_chained_fast_local_assignment_ownership() {
    for source in [
        "def f(value):\n    left = right = value\n    return left, right\n",
        "def f(value):\n    left = middle = right = value\n    return left, middle, right\n",
        "def f(value):\n    left = right = value\n    return left, lambda: right\n",
        "def f(value):\n    left = right = value\n    return right, lambda: left\n",
        "def f(value):\n    value = left = value\n    return lambda: left\n",
    ] {
        common::assert_matches_cpython_marshal(
            source,
            "chained_fast_local_assignment_ownership.py",
        );
    }
}

#[test]
fn async_comprehension_branch_join_simple() {
    let source = r"async def f(flag, stream):
    if flag:
        result = []
    else:
        result = [item async for item in stream]
    return consume(item for item in result)
";
    common::assert_matches_cpython_marshal(source, "async_comprehension_branch_join_simple.py");
}

#[test]
fn async_comprehension_branch_join_protected_loop() {
    let source = r#"async def f(manager, items, engine):
    with manager:
        with manager:
            for index in items:
                result = []
                if index == 0:
                    value = engine.read()
                    result = [value]
                elif index == 1:
                    value = engine.read()
                    result = list(value)
                elif index == 2:
                    value = await engine.read()
                    result = [value]
                else:
                    value = await engine.read()
                    result = [item async for item in value]
                text = "".join(item[0] for item in result)
                assert text == "value"
"#;
    common::assert_matches_cpython_marshal(
        source,
        "async_comprehension_branch_join_protected_loop.py",
    );
}

#[test]
fn async_comprehension_branch_join_straight_line_control() {
    let source = r"async def f(stream):
    result = [item async for item in stream]
    return consume(item for item in result)
";
    common::assert_matches_cpython_marshal(source, "async_comprehension_straight_line_control.py");
}

#[test]
fn async_comprehension_branch_join_first_arm() {
    let source = r"async def f(flag, stream):
    if flag:
        result = [item async for item in stream]
    else:
        result = []
    return consume(item for item in result)
";
    common::assert_matches_cpython_marshal(source, "async_comprehension_first_arm.py");
}

#[test]
fn async_comprehension_branch_join_both_arms_control() {
    let source = r"async def f(flag, stream):
    if flag:
        result = [item async for item in stream]
    else:
        result = [other async for other in stream]
    return consume(item for item in result)
";
    common::assert_matches_cpython_marshal(source, "async_comprehension_both_arms_control.py");
}

#[test]
fn async_comprehension_branch_join_sync_outer_control() {
    let source = r"async def f(items, stream):
    result = [item for group in items async for item in stream]
    return consume(item for item in result)
";
    common::assert_matches_cpython_marshal(source, "async_comprehension_sync_outer_control.py");
}

#[test]
fn async_for_boolean_tuple_simple_exit() {
    common::assert_matches_cpython_marshal(
        "async def f(items, status, domain):\n    async for item in items:\n        if item:\n            break\n    return status or \"\", domain\n",
        "async_for_boolean_tuple_simple_exit.py",
    );
}

#[test]
fn async_for_boolean_tuple_conditional_exit() {
    common::assert_matches_cpython_marshal(
        r#"async def f(items):
    status = None
    domain = None
    found = False
    async for item in items:
        if item.skip:
            if item.stop:
                break
            continue
        if status is None:
            status = await item.label()
        if not found and item.accepted:
            found = True
            domain = await item.domain()
        if status is not None and found:
            break
    return status or "", domain
"#,
        "async_for_boolean_tuple_conditional_exit.py",
    );
}

#[test]
fn async_for_boolean_tuple_continue_only_exit() {
    common::assert_matches_cpython_marshal(
        r#"async def f(items, status, domain):
    async for item in items:
        if item.skip:
            continue
        if item.stop:
            break
    return status or "", domain
"#,
        "async_for_boolean_tuple_continue_only_exit.py",
    );
}

#[test]
fn keeps_owned_loads_after_nested_conditional_try_recovery() {
    common::assert_matches_cpython_marshal(
        r#"def f(items, load, parse):
    total = 0
    out = {}
    for key in items:
        try:
            record = load(key)
            good = int(record.good or 0)
            bad = int(record.bad or 0)
            if good == 0 and bad == 0:
                try:
                    other = parse(key)
                    if isinstance(other, dict) and "count" in other:
                        good = int(other.get("count") or 0)
                except Exception:
                    pass
        except Exception:
            good = 0
            bad = 0
        out[key] = (good, bad)
        total += good
    return out, total
"#,
        "nested_conditional_try_recovery.py",
    );
}

#[test]
fn post_finally_cleanup_handlers_preserve_owned_tail() {
    let source = r"def f(value, emit):
    handle = None
    path = None
    try:
        handle = acquire()
        path = prepare(value)
        work(handle, path)
    finally:
        if handle is not None:
            try:
                release(handle)
            except Error:
                pass
        if path is not None and path.exists():
            try:
                path.remove()
            except Error:
                pass
    emit(value)
    return value
";
    common::assert_matches_cpython_marshal(source, "post_finally_cleanup_handlers.py");
}

#[test]
fn matches_borrow_entries_with_nested_conditional_recovery_alternatives() {
    for (name, tail) in [
        (
            "single_arm",
            "if choose(other):\n                        good = left(other)",
        ),
        (
            "both_arms",
            "if choose(other):\n                        good = left(other)\n                    else:\n                        good = right(other)",
        ),
    ] {
        let source = format!(
            "def f(items, load, parse, choose, left, right):\n    total = 0\n    out = {{}}\n    for key in items:\n        try:\n            record = load(key)\n            good = int(record.good or 0)\n            bad = int(record.bad or 0)\n            if good == 0 and bad == 0:\n                try:\n                    other = parse(key)\n                    {tail}\n                except Exception:\n                    pass\n        except Exception:\n            good = 0\n            bad = 0\n        out[key] = (good, bad)\n        total += good\n    return out, total\n"
        );
        common::assert_matches_cpython_marshal(&source, &format!("nested_recovery_{name}.py"));
    }
}

#[test]
fn keeps_owned_loads_after_nested_finalizer_in_named_recovery() {
    common::assert_matches_cpython_marshal(
        r"def f(items, action, should_reraise, restore):
    try:
        values = [action(item) for item in items]
    except RuntimeError as error:
        if should_reraise(error):
            raise
        try:
            values = [action(item) for item in items]
        finally:
            restore()
    result = [(index, sum(values)) for index in range(len(values))]
    return result, items
",
        "nested_finalizer_named_recovery.py",
    );
}

#[test]
fn matches_nested_finalizer_recovery_borrow_entry_controls() {
    common::assert_matches_cpython_marshal(
        r"def f(items, action, should_reraise, restore):
    try:
        values = [action(item) for item in items]
    except RuntimeError as error:
        try:
            values = [action(item) for item in items]
        finally:
            restore()
    result = [(index, sum(values)) for index in range(len(values))]
    return result, items
",
        "nested_finalizer_named_without_guard.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(items, action, should_reraise, restore):
    try:
        values = [action(item) for item in items]
    except RuntimeError:
        if should_reraise(items):
            raise
        try:
            values = [action(item) for item in items]
        finally:
            restore()
    result = [(index, sum(values)) for index in range(len(values))]
    return result, items
",
        "nested_finalizer_unnamed_recovery.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(items, action, should_reraise, restore, flag):
    if flag:
        try:
            values = [action(item) for item in items]
        except RuntimeError as error:
            if should_reraise(error):
                raise
            try:
                values = [action(item) for item in items]
            finally:
                restore()
    else:
        values = []
    result = [(index, sum(values)) for index in range(len(values))]
    return result, items
",
        "nested_finalizer_independent_branch_entry.py",
    );
}

#[test]
fn post_try_terminal_else_break_preserves_owned_await_tail() {
    let source = r"async def f(value, emit):
    try:
        while True:
            item = read()
            if keep(item):
                record(item)
            else:
                finish(item)
                break
    except Error as error:
        record(error)
        pass
    await emit(value)
";
    common::assert_matches_cpython_marshal(source, "post_try_terminal_else_break.py");
}

#[test]
fn post_try_terminal_else_break_independent_early_break_entry() {
    let source = r"async def f(stop, flag, value, emit):
    try:
        while True:
            if stop:
                break
            if flag:
                tick()
            else:
                tock()
                break
    except Error as error:
        record(error)
        pass
    await emit(value)
";
    common::assert_matches_cpython_marshal(source, "post_try_early_break_entry.py");
}

#[test]
fn post_try_terminal_else_break_opposite_arm_entry() {
    let source = r"async def f(flag, value, emit):
    try:
        while True:
            if flag:
                tick()
                break
            else:
                tock()
    except Error as error:
        record(error)
        pass
    await emit(value)
";
    common::assert_matches_cpython_marshal(source, "post_try_then_break_entry.py");
}

#[test]
fn post_try_terminal_else_break_synchronous_owned_tail() {
    let source = r"def f(value, emit):
    try:
        while True:
            item = read()
            if keep(item):
                record(item)
            else:
                finish(item)
                break
    except Error as error:
        record(error)
        pass
    emit(value)
";
    common::assert_matches_cpython_marshal(source, "post_try_terminal_else_break_sync.py");
}

#[test]
fn post_try_terminal_else_break_independent_elif_entry() {
    let source = r"async def f(flag, stop, value, emit):
    try:
        while True:
            if flag:
                tick()
            elif stop:
                break
            else:
                tock()
                break
    except Error as error:
        record(error)
        pass
    await emit(value)
";
    common::assert_matches_cpython_marshal(source, "post_try_elif_break_entry.py");
}

#[test]
fn nested_finally_after_while_recovery_preserves_borrowed_cleanup() {
    common::assert_matches_cpython_marshal(
        r"def f(self):
    terminal_error = None
    try:
        self.manager.started()
        self.ready.set()
        while not self.stop.is_set():
            self.manager.heartbeat()
            candidates = self.manager.candidates()
            for key, date in candidates:
                if self.stop.is_set():
                    break
                self.manager.refresh(key, date=date)
            self.wake.wait()
            self.wake.clear()
    except BaseException as error:
        self.error = error
        terminal_error = str(error)
        self.ready.set()
    finally:
        try:
            self.manager.stopped(terminal_error)
        finally:
            self.lock.release()
            self.manager.finished(self)
",
        "nested_finally_while_recovery.py",
    );
}

#[test]
fn matches_terminal_finalizer_recovery_borrow_controls() {
    common::assert_matches_cpython_marshal(
        r"def f(items, action, should_reraise, restore):
    try:
        values = [action(item) for item in items]
    except RuntimeError as error:
        if should_reraise(error):
            raise
        try:
            values = [action(item) for item in items]
        finally:
            return items
    result = [(index, sum(values)) for index in range(len(values))]
    return result, items
",
        "terminal_finalizer_recovery_return.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(items, action, should_reraise, restore):
    try:
        values = [action(item) for item in items]
    except RuntimeError as error:
        if should_reraise(error):
            raise
        try:
            values = [action(item) for item in items]
        finally:
            raise RuntimeError
    result = [(index, sum(values)) for index in range(len(values))]
    return result, items
",
        "terminal_finalizer_recovery_raise.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(items, action, should_reraise, restore):
    try:
        values = [action(item) for item in items]
    except RuntimeError as error:
        if should_reraise(error):
            raise
        try:
            values = [action(item) for item in items]
        finally:
            pass
    result = [(index, sum(values)) for index in range(len(values))]
    return result, items
",
        "terminal_finalizer_recovery_pass.py",
    );
}

#[test]
fn async_for_finally_manager_preserves_owned_continuation() {
    common::assert_matches_cpython_marshal(
        r"async def f(client, terminal, worker, events, value, choose, emit):
    try:
        async with client:
            with terminal:
                worker.start()
                try:
                    async for event in events:
                        terminal.update(event)
                        if event.changed:
                            status = event.status
                            if choose(status if isinstance(status, str) else None):
                                break
                finally:
                    cleanup.cancel()
                    await cleanup.wait()
            emit(value.first)
            emit(value.second)
    finally:
        await worker.stop()
    emit(value.third)
",
        "async_for_finally_manager_global_cleanup.py",
    );
}

#[test]
fn keeps_owned_loads_after_finalizer_with_nested_recovery() {
    common::assert_matches_cpython_marshal(
        r"def f(open_stream, proc):
    selected = None
    try:
        with open_stream() as stream:
            for value in stream:
                if value:
                    selected = value
                    break
    finally:
        if selected is not None and proc.poll() is None:
            proc.terminate()
        if proc.stdout:
            proc.stdout.close()
        try:
            _, stderr = proc.communicate()
        except RuntimeError:
            proc.kill()
            _, stderr = proc.communicate()
        code = proc.code
    if selected is None and code != 0:
        message = stderr.decode() if stderr else str(code)
        raise RuntimeError(message, code)
    return selected
",
        "finalizer_nested_recovery_tail.py",
    );
}

#[test]
fn reducer_finalizer_recovery_linear_protected_body() {
    common::assert_matches_cpython_marshal(
        r"def f(open_stream, proc):
    selected = None
    try:
        selected = open_stream()
    finally:
        if selected is not None and proc.poll() is None:
            proc.terminate()
        if proc.stdout:
            proc.stdout.close()
        try:
            _, stderr = proc.communicate()
        except RuntimeError:
            proc.kill()
            _, stderr = proc.communicate()
        code = proc.code
    if selected is None and code != 0:
        message = stderr.decode() if stderr else str(code)
        raise RuntimeError(message, code)
    return selected
",
        "reducer_finalizer_recovery_linear_protected_body.py",
    );
}

#[test]
fn reducer_finalizer_recovery_no_leading_conditions() {
    common::assert_matches_cpython_marshal(
        r"def f(open_stream, proc):
    selected = None
    try:
        with open_stream() as stream:
            for value in stream:
                if value:
                    selected = value
                    break
    finally:
        try:
            _, stderr = proc.communicate()
        except RuntimeError:
            proc.kill()
            _, stderr = proc.communicate()
        code = proc.code
    if selected is None and code != 0:
        message = stderr.decode() if stderr else str(code)
        raise RuntimeError(message, code)
    return selected
",
        "reducer_finalizer_recovery_no_leading_conditions.py",
    );
}

#[test]
fn reducer_finalizer_recovery_no_nested_recovery() {
    common::assert_matches_cpython_marshal(
        r"def f(open_stream, proc):
    selected = None
    try:
        with open_stream() as stream:
            for value in stream:
                if value:
                    selected = value
                    break
    finally:
        if selected is not None and proc.poll() is None:
            proc.terminate()
        if proc.stdout:
            proc.stdout.close()
        _, stderr = proc.communicate()
        code = proc.code
    if selected is None and code != 0:
        message = stderr.decode() if stderr else str(code)
        raise RuntimeError(message, code)
    return selected
",
        "reducer_finalizer_recovery_no_nested_recovery.py",
    );
}

#[test]
fn reducer_finalizer_copy_tail_long_attr() {
    common::assert_matches_cpython_marshal(
        r"def f(open_stream, proc):
    selected = None
    try:
        with open_stream() as stream:
            for value in stream:
                if value:
                    selected = value
                    break
    finally:
        try:
            _, stderr = proc.communicate()
        except RuntimeError:
            proc.kill()
            _, stderr = proc.communicate()
        code = proc.inner.code
    if selected is None and code != 0:
        message = stderr.decode() if stderr else str(code)
        raise RuntimeError(message, code)
    return selected
",
        "reducer_finalizer_copy_tail_long_attr.py",
    );
}

#[test]
fn reducer_finalizer_copy_tail_binary() {
    common::assert_matches_cpython_marshal(
        r"def f(open_stream, proc):
    selected = None
    try:
        with open_stream() as stream:
            for value in stream:
                if value:
                    selected = value
                    break
    finally:
        try:
            _, stderr = proc.communicate()
        except RuntimeError:
            proc.kill()
            _, stderr = proc.communicate()
        code = proc.code + 1
    if selected is None and code != 0:
        message = stderr.decode() if stderr else str(code)
        raise RuntimeError(message, code)
    return selected
",
        "reducer_finalizer_copy_tail_binary.py",
    );
}

#[test]
fn reducer_finalizer_copy_tail_short_const() {
    common::assert_matches_cpython_marshal(
        r"def f(open_stream, proc):
    selected = None
    try:
        with open_stream() as stream:
            for value in stream:
                if value:
                    selected = value
                    break
    finally:
        try:
            _, stderr = proc.communicate()
        except RuntimeError:
            proc.kill()
            _, stderr = proc.communicate()
        code = 0
    if selected is None and code != 0:
        message = stderr.decode() if stderr else str(code)
        raise RuntimeError(message, code)
    return selected
",
        "reducer_finalizer_copy_tail_short_const.py",
    );
}

#[test]
fn finalizer_copy_certificate_pass_nops() {
    common::assert_matches_cpython_marshal(
        r"def f(open_stream, proc):
    selected = None
    try:
        with open_stream() as stream:
            for value in stream:
                if value:
                    selected = value
                    break
    finally:
        try:
            _, stderr = proc.communicate()
        except RuntimeError:
            proc.kill()
            _, stderr = proc.communicate()
        code = 0
        pass
        pass
    if selected is None and code != 0:
        message = stderr.decode() if stderr else str(code)
        raise RuntimeError(message, code)
    return selected
",
        "finalizer_copy_certificate_pass_nops.py",
    );
}

#[test]
fn finalizer_copy_certificate_constant_nops() {
    common::assert_matches_cpython_marshal(
        r"def f(open_stream, proc):
    selected = None
    try:
        with open_stream() as stream:
            for value in stream:
                if value:
                    selected = value
                    break
    finally:
        try:
            _, stderr = proc.communicate()
        except RuntimeError:
            proc.kill()
            _, stderr = proc.communicate()
        code = 0
        1
        2
    if selected is None and code != 0:
        message = stderr.decode() if stderr else str(code)
        raise RuntimeError(message, code)
    return selected
",
        "finalizer_copy_certificate_constant_nops.py",
    );
}

#[test]
fn finalizer_copy_certificate_tuple_before_fold() {
    common::assert_matches_cpython_marshal(
        r"def f(open_stream, proc):
    selected = None
    try:
        with open_stream() as stream:
            for value in stream:
                if value:
                    selected = value
                    break
    finally:
        try:
            _, stderr = proc.communicate()
        except RuntimeError:
            proc.kill()
            _, stderr = proc.communicate()
        code = (1, 2, 3)
    if selected is None and code != 0:
        message = stderr.decode() if stderr else str(code)
        raise RuntimeError(message, code)
    return selected
",
        "finalizer_copy_certificate_tuple_before_fold.py",
    );
}

#[test]
fn finalizer_copy_certificate_await_assignment() {
    common::assert_matches_cpython_marshal(
        r"async def f(open_stream, proc):
    selected = None
    try:
        with open_stream() as stream:
            for value in stream:
                if value:
                    selected = value
                    break
    finally:
        try:
            _, stderr = proc.communicate()
        except RuntimeError:
            proc.kill()
            _, stderr = proc.communicate()
        code = await proc.finish()
    if selected is None and code != 0:
        message = stderr.decode() if stderr else str(code)
        raise RuntimeError(message, code)
    return selected
",
        "finalizer_copy_certificate_await_assignment.py",
    );
}

#[test]
fn finalizer_copy_certificate_yield_assignment() {
    common::assert_matches_cpython_marshal(
        r"def f(open_stream, proc):
    selected = None
    try:
        with open_stream() as stream:
            for value in stream:
                if value:
                    selected = value
                    break
    finally:
        try:
            _, stderr = proc.communicate()
        except RuntimeError:
            proc.kill()
            _, stderr = proc.communicate()
        code = yield proc.code
    if selected is None and code != 0:
        message = stderr.decode() if stderr else str(code)
        raise RuntimeError(message, code)
    return selected
",
        "finalizer_copy_certificate_yield_assignment.py",
    );
}

#[test]
fn finalizer_copy_certificate_yield_from_assignment() {
    common::assert_matches_cpython_marshal(
        r"def f(open_stream, proc):
    selected = None
    try:
        with open_stream() as stream:
            for value in stream:
                if value:
                    selected = value
                    break
    finally:
        try:
            _, stderr = proc.communicate()
        except RuntimeError:
            proc.kill()
            _, stderr = proc.communicate()
        code = yield from proc.values
    if selected is None and code != 0:
        message = stderr.decode() if stderr else str(code)
        raise RuntimeError(message, code)
    return selected
",
        "finalizer_copy_certificate_yield_from_assignment.py",
    );
}

#[test]
fn finalizer_copy_certificate_enclosing_try_constant_nops() {
    common::assert_matches_cpython_marshal(
        r"def f(open_stream, proc):
    try:
        selected = None
        try:
            with open_stream() as stream:
                for value in stream:
                    if value:
                        selected = value
                        break
        finally:
            try:
                _, stderr = proc.communicate()
            except RuntimeError:
                proc.kill()
                _, stderr = proc.communicate()
            code = 0
            1
            2
        if selected is None and code != 0:
            message = stderr.decode() if stderr else str(code)
            raise RuntimeError(message, code)
        return selected
    finally:
        proc.close()
",
        "finalizer_copy_certificate_enclosing_try_constant_nops.py",
    );
}

#[test]
fn normal_finally_retry_break_preserves_owned_continuation() {
    let source = r"async def f(value, emit):
    task = None
    result = None
    error = None
    try:
        try:
            task = await start(value)
        except BaseError as exc:
            error = exc
            task = recover(value)
    finally:
        if task is not None:
            while True:
                try:
                    result = await shield(task)
                    break
                except Cancel as exc:
                    if task.cancelled():
                        raise
                    if error is None:
                        error = exc
                except Error as exc:
                    if error is not None:
                        raise error from exc
                    raise
    if error is not None:
        raise error
    assert result is not None
    emit(result)
    return result
";
    common::assert_matches_cpython_marshal(source, "normal_finally_retry_break.py");
}

#[test]
fn normal_finally_retry_break_same_line() {
    let source = r"async def f(value, emit):
    task = None
    result = None
    error = None
    try:
        try:
            task = await start(value)
        except BaseError as exc:
            error = exc
            task = recover(value)
    finally:
        if task is not None:
            while True:
                try:
                    result = await shield(task); break
                except Cancel as exc:
                    if task.cancelled():
                        raise
                    if error is None:
                        error = exc
                except Error as exc:
                    if error is not None:
                        raise error from exc
                    raise
    if error is not None:
        raise error
    assert result is not None
    emit(result)
    return result
";
    common::assert_matches_cpython_marshal(source, "normal_finally_retry_break_same_line.py");
}

#[test]
fn normal_finally_retry_break_independent_early_entry() {
    let source = r"async def f(value, emit, done):
    task = None
    result = None
    error = None
    try:
        try:
            task = await start(value)
        except BaseError as exc:
            error = exc
            task = recover(value)
    finally:
        if task is not None:
            while True:
                if done:
                    break
                try:
                    result = await shield(task)
                    break
                except Cancel as exc:
                    if task.cancelled():
                        raise
                    if error is None:
                        error = exc
                except Error as exc:
                    if error is not None:
                        raise error from exc
                    raise
    if error is not None:
        raise error
    assert result is not None
    emit(result)
    return result
";
    common::assert_matches_cpython_marshal(
        source,
        "normal_finally_retry_break_independent_entry.py",
    );
}

#[test]
fn normal_finally_retry_break_outer_loop() {
    let source = r"async def f(value, emit):
    task = None
    result = None
    error = None
    while again():
        try:
            try:
                task = await start(value)
            except BaseError as exc:
                error = exc
                task = recover(value)
        finally:
            if task is not None:
                while True:
                    try:
                        result = await shield(task)
                        break
                    except Cancel as exc:
                        if task.cancelled():
                            raise
                        if error is None:
                            error = exc
                    except Error as exc:
                        if error is not None:
                            raise error from exc
                        raise
    if error is not None:
        raise error
    assert result is not None
    emit(result)
    return result
";
    common::assert_matches_cpython_marshal(source, "normal_finally_retry_break_outer_loop.py");
}

#[test]
fn normal_finally_retry_break_located_cleanup() {
    let source = r"async def f(value, emit):
    task = None
    result = None
    error = None
    try:
        try:
            task = await start(value)
        except BaseError as exc:
            error = exc
            task = recover(value)
    finally:
        if task is not None:
            while True:
                try:
                    result = await shield(task)
                    break
                except Cancel as exc:
                    if task.cancelled():
                        raise
                    if error is None:
                        error = exc
                except Error as exc:
                    if error is not None:
                        raise error from exc
                    raise
            emit(result)
    if error is not None:
        raise error
    assert result is not None
    emit(result)
    return result
";
    common::assert_matches_cpython_marshal(source, "normal_finally_retry_break_located_cleanup.py");
}

#[test]
fn normal_finally_conditional_recovery_boolean_cleanup() {
    let source = r"def f(factory, exists, emit, value):
    output = None
    published = False
    try:
        output = factory()
        emit(output, value)
        output.close()
        published = True
    except Error:
        if not exists():
            raise
        published = True
    finally:
        if output is not None and not published:
            output.cancel()
    emit(output, value)
    return value, output
";
    common::assert_matches_cpython_marshal(
        source,
        "normal_finally_conditional_recovery_boolean_cleanup.py",
    );
}

#[test]
fn normal_finally_conditional_recovery_single_condition_cleanup() {
    let source = r"def f(factory, exists, emit, value):
    output = None
    published = False
    try:
        output = factory()
        emit(output, value)
        output.close()
        published = True
    except Error:
        if not exists():
            raise
        published = True
    finally:
        if output is not None:
            output.cancel()
    emit(output, value)
    return value, output
";
    common::assert_matches_cpython_marshal(
        source,
        "normal_finally_conditional_recovery_single_condition_cleanup.py",
    );
}

#[test]
fn normal_finally_conditional_recovery_no_recovery_suffix() {
    let source = r"def f(factory, exists, emit, value):
    output = None
    published = False
    try:
        output = factory()
        emit(output, value)
        output.close()
        published = True
    except Error:
        if not exists():
            raise
    finally:
        if output is not None and not published:
            output.cancel()
    emit(output, value)
    return value, output
";
    common::assert_matches_cpython_marshal(
        source,
        "normal_finally_conditional_recovery_no_recovery_suffix.py",
    );
}

#[test]
fn normal_finally_conditional_recovery_handler_controls() {
    common::assert_matches_cpython_marshal(
        r"def f(factory, exists, emit, value):
    output = None
    published = False
    try:
        output = factory()
        emit(output, value)
        output.close()
        published = True
    except Error as error:
        if not exists():
            raise
        published = True
    finally:
        if output is not None and not published:
            output.cancel()
    emit(output, value)
    return value, output
",
        "normal_finally_conditional_recovery_named_boolean_cleanup.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(factory, exists, emit, value):
    output = None
    published = False
    try:
        output = factory()
        emit(output, value)
        output.close()
        published = True
    except Error:
        raise
    finally:
        if output is not None and not published:
            output.cancel()
    emit(output, value)
    return value, output
",
        "normal_finally_conditional_recovery_terminal_handler.py",
    );
}

#[test]
fn runtime_while_handler_cleanup_shape_both_conditional() {
    let source = r#"def fill(state):
    try:
        while not state.stopped():
            try:
                item = next(state.iterator)
            except StopIteration:
                state.buffer.put(STOP)
                break
            state.buffer.put(item)
    except Exception as error:
        if not state.stopped():
            state.buffer.put(wrap(error))
            state.buffer.put(STOP)
    finally:
        if hasattr(state.iterator, "close"):
            state.iterator.close()
"#;
    common::assert_matches_cpython_marshal(source, "runtime_while_handler_cleanup_shape.py");
}

#[test]
fn runtime_while_handler_cleanup_shape_linear_handler() {
    let source = r#"def fill(state):
    try:
        while not state.stopped():
            try:
                item = next(state.iterator)
            except StopIteration:
                state.buffer.put(STOP)
                break
            state.buffer.put(item)
    except Exception as error:
        state.buffer.put(wrap(error))
        state.buffer.put(STOP)
    finally:
        if hasattr(state.iterator, "close"):
            state.iterator.close()
"#;
    common::assert_matches_cpython_marshal(source, "runtime_while_handler_cleanup_shape.py");
}

#[test]
fn runtime_while_handler_cleanup_shape_linear_cleanup() {
    let source = r"def fill(state):
    try:
        while not state.stopped():
            try:
                item = next(state.iterator)
            except StopIteration:
                state.buffer.put(STOP)
                break
            state.buffer.put(item)
    except Exception as error:
        if not state.stopped():
            state.buffer.put(wrap(error))
            state.buffer.put(STOP)
    finally:
        state.iterator.close()
";
    common::assert_matches_cpython_marshal(source, "runtime_while_handler_cleanup_shape.py");
}

#[test]
fn runtime_while_handler_cleanup_shape_both_linear() {
    let source = r"def fill(state):
    try:
        while not state.stopped():
            try:
                item = next(state.iterator)
            except StopIteration:
                state.buffer.put(STOP)
                break
            state.buffer.put(item)
    except Exception as error:
        state.buffer.put(wrap(error))
        state.buffer.put(STOP)
    finally:
        state.iterator.close()
";
    common::assert_matches_cpython_marshal(source, "runtime_while_handler_cleanup_shape.py");
}

#[test]
fn terminal_handler_borrow_named_then_unnamed_asserts() {
    common::assert_matches_cpython_marshal(
        r"def f(run, values):
    try:
        run()
    except ValueError as error:
        assert False, f'bad: {error}'
    except TimeoutError:
        assert False, 'late'
    found = [value for value in values if value]
    assert found, 'empty'
    return values, found
",
        "terminal_handler_borrow_named_then_unnamed_asserts.py",
    );
}

#[test]
fn terminal_handler_borrow_single_named_assert() {
    common::assert_matches_cpython_marshal(
        r"def f(run, values):
    try:
        run()
    except ValueError as error:
        assert False, f'bad: {error}'
    found = [value for value in values if value]
    assert found, 'empty'
    return values, found
",
        "terminal_handler_borrow_single_named_assert.py",
    );
}

#[test]
fn terminal_handler_borrow_explicit_raise_handlers() {
    common::assert_matches_cpython_marshal(
        r"def f(run, values):
    try:
        run()
    except ValueError as error:
        raise RuntimeError(f'bad: {error}')
    except TimeoutError:
        raise RuntimeError('late')
    found = [value for value in values if value]
    assert found, 'empty'
    return values, found
",
        "terminal_handler_borrow_explicit_raise_handlers.py",
    );
}

#[test]
fn terminal_handler_borrow_mixed_live_handler() {
    common::assert_matches_cpython_marshal(
        r"def f(run, values):
    try:
        run()
    except ValueError as error:
        assert False, f'bad: {error}'
    except TimeoutError:
        values = []
    found = [value for value in values if value]
    assert found, 'empty'
    return values, found
",
        "terminal_handler_borrow_mixed_live_handler.py",
    );
}

#[test]
fn terminal_handler_borrow_following_name_return() {
    common::assert_matches_cpython_marshal(
        r"def f(run, values):
    try:
        run()
    except ValueError as error:
        assert False, f'bad: {error}'
    except TimeoutError:
        assert False, 'late'
    return values
",
        "terminal_handler_borrow_following_name_return.py",
    );
}

#[test]
fn terminal_handler_borrow_mixed_false_last() {
    common::assert_matches_cpython_marshal(
        r"def f(run, values):
    try:
        run()
    except ValueError as error:
        values = [error]
    except TimeoutError:
        assert False, 'late'
    found = [value for value in values if value]
    assert found, 'empty'
    return values, found
",
        "terminal_handler_borrow_mixed_false_last.py",
    );
}

#[test]
fn terminal_handler_borrow_all_live_handlers() {
    common::assert_matches_cpython_marshal(
        r"def f(run, values):
    try:
        run()
    except ValueError as error:
        values = [error]
    except TimeoutError:
        values = []
    found = [value for value in values if value]
    assert found, 'empty'
    return values, found
",
        "terminal_handler_borrow_all_live_handlers.py",
    );
}

#[test]
fn terminal_if_return_false_edge_reaches_normal_finally() {
    for handler in [
        "except Error:\n            pass",
        "except Error as error:\n            recover(error)",
    ] {
        for cleanup in [
            "sink.close()",
            "if sink is not None:\n                sink.close()",
        ] {
            let source = format!(
                "def f(sink, expected):\n    while ready():\n        try:\n            result = work()\n            if result == expected:\n                return\n        {handler}\n        finally:\n            {cleanup}\n        again()\n",
            );
            common::assert_matches_cpython_marshal(&source, "terminal_if_return_finally.py");
        }
    }
}

fn check_terminal_if_return_finally_condition(condition: &str) {
    let source = format!(
        "def f(sink, left, right):\n    try:\n        if {condition}:\n            return\n    except Error:\n        pass\n    finally:\n        sink.close()\n    again(sink)\n",
    );
    common::assert_matches_cpython_marshal(&source, "terminal_if_return_finally_control.py");
}

#[test]
fn terminal_if_return_finally_shared_condition_control() {
    check_terminal_if_return_finally_condition("left and right");
}

#[test]
fn terminal_if_return_finally_single_false_edge_condition_control() {
    check_terminal_if_return_finally_condition("left or right");
}

#[test]
fn terminal_if_return_finally_folded_condition_control() {
    check_terminal_if_return_finally_condition("left and True");
}

#[test]
fn conditional_recovery_reaches_linear_finally_entry_before_branch() {
    for prefix in ["", "async "] {
        for leading in ["elapsed = clock() - started", "sink.record(value)"] {
            let source = format!(
                "{prefix}def f(sink, value, suppress, started):\n    try:\n        work(value)\n    except Error as error:\n        recover(error)\n        if not suppress:\n            raise\n    finally:\n        {leading}\n        if value is not None:\n            sink.close(value)\n    after(sink)\n",
            );
            common::assert_matches_cpython_marshal(
                &source,
                "conditional_recovery_finally_entry.py",
            );
        }
    }
}

fn check_conditional_recovery_cleanup_entry(cleanup: &str) {
    let source = format!(
        "def f(sink, value, suppress):\n    try:\n        work(value)\n    except Error as error:\n        recover(error)\n        if not suppress:\n            raise\n    finally:\n        {cleanup}\n    after(sink)\n",
    );
    common::assert_matches_cpython_marshal(
        &source,
        "conditional_recovery_finally_entry_control.py",
    );
}

#[test]
fn conditional_recovery_preserves_leading_pass_cleanup_entry() {
    check_conditional_recovery_cleanup_entry(
        "pass\n        if value:\n            sink.close(value)",
    );
}

#[test]
fn conditional_recovery_preserves_abrupt_cleanup_entry() {
    check_conditional_recovery_cleanup_entry(
        "sink.record(value)\n        if value:\n            return value",
    );
}

#[test]
fn conditional_retry_handler_reaches_normal_finally() {
    for prefix in ["", "async "] {
        let awaited = if prefix.is_empty() { "" } else { "await " };
        for condition in [
            "retry",
            "retry and error.retryable() and item.attempts < limit",
        ] {
            let source = format!(
                "{prefix}def f(sink, item, retry, limit):\n    while ready():\n        ack = True\n        try:\n            {awaited}work(item)\n        except Error as error:\n            item.attempts += 1\n            record(error)\n            if {condition}:\n                {awaited}reschedule(item)\n                ack = False\n                continue\n            failed(item)\n        finally:\n            sink.record(item)\n            if ack:\n                {awaited}sink.ack(item)\n            sink.done()\n    after(sink)\n",
            );
            common::assert_matches_cpython_marshal(&source, "conditional_retry_handler_finally.py");
        }
    }
}

#[test]
fn conditional_retry_handler_without_recovery_suffix() {
    common::assert_matches_cpython_marshal(
        "def f(sink, item, retry):\n    while ready():\n        try:\n            work(item)\n        except Error as error:\n            record(error)\n            if retry:\n                reschedule(item)\n                continue\n        finally:\n            sink.record(item)\n            sink.done()\n    after(sink)\n",
        "conditional_retry_without_suffix.py",
    );
}

#[test]
fn conditional_retry_handler_break_keeps_existing_entry() {
    common::assert_matches_cpython_marshal(
        "def f(sink, item, retry):\n    while ready():\n        try:\n            work(item)\n        except Error as error:\n            record(error)\n            if retry:\n                reschedule(item)\n                break\n            failed(item)\n        finally:\n            sink.record(item)\n            sink.done()\n    after(sink)\n",
        "conditional_retry_break_entry.py",
    );
}

#[test]
fn conditional_retry_logging_join_reaches_normal_finally() {
    for prefix in ["", "async "] {
        let awaited = if prefix.is_empty() { "" } else { "await " };
        let source = format!(
            "{prefix}def f(sink, item, retry, limit):\n    while ready():\n        ack = True\n        try:\n            {awaited}work(item)\n        except Error as error:\n            item.attempts += 1\n            record(error)\n            if retry and error.retryable() and item.attempts < limit:\n                delay = error.delay(item.attempts)\n                {awaited}reschedule(item, delay)\n                if item.attempts == 1 or item.attempts % 5 == 0:\n                    record(item, delay)\n                ack = False\n                continue\n            failed(item)\n        finally:\n            sink.record(item)\n            if ack:\n                {awaited}sink.ack(item)\n            sink.done()\n    after(sink)\n",
        );
        common::assert_matches_cpython_marshal(&source, "conditional_retry_logging_finally.py");
    }
}

#[test]
fn nested_raising_handler_keeps_finally_entry_unborrowed() {
    for leading in ["", "        prepare(sink)\n"] {
        for raised in ["raise", "raise Error(value) from None"] {
            let source = format!(
                "def f(sink, flag, work, record, after, prepare):\n    try:\n{leading}        try:\n            value = work()\n            if flag is not None:\n                record(value)\n        except Error as error:\n            if flag is not None:\n                record(error)\n            {raised}\n    finally:\n        sink.close()\n    after(sink, value)\n"
            );
            common::assert_matches_cpython_marshal(&source, "nested_raising_finally.py");
        }
    }
}

#[test]
fn nested_warning_handler_keeps_finally_entry_unborrowed() {
    for warning in ["warn(error)", "logger.warning('failed', describe(error))"] {
        let source = format!(
            "def f(sink, flag, work, record, after):\n    try:\n        try:\n            value = work()\n            if flag is not None:\n                record(value)\n        except Error as error:\n            if flag is not None:\n                record(error)\n            {warning}\n    finally:\n        sink.close()\n    after(sink)\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_warning_finally.py");
    }
}

#[test]
fn dispatcher_break_reaches_linear_finally_after_structured_arm() {
    common::assert_matches_cpython_marshal(
        "def f(sink, value, manager):\n    try:\n        while True:\n            command = receive()\n            if command == 'work':\n                work(value)\n            elif command == 'stop':\n                break\n            elif command == 'write':\n                with manager:\n                    write(value)\n    except Interrupt:\n        pass\n    except Error as error:\n        report(error)\n    finally:\n        sink.close()\n",
        "dispatcher_break_structured_arm.py",
    );
}

#[test]
fn dispatcher_break_with_linear_prefix_reaches_finally() {
    common::assert_matches_cpython_marshal(
        "def f(sink, value):\n    try:\n        while True:\n            command, data = receive()\n            if command == 'work':\n                work(data)\n            elif command == 'stop':\n                reply(value)\n                break\n            else:\n                raise Error(command)\n    except (Interrupt, Error):\n        report(value)\n    finally:\n        sink.close()\n",
        "dispatcher_break_linear_prefix.py",
    );
}

#[test]
fn dispatcher_break_reaches_conditional_cleanup_with_raising_handler() {
    common::assert_matches_cpython_marshal(
        "def f(sink, value):\n    try:\n        setup(value)\n        while True:\n            command = receive()\n            if command == 'work':\n                work(value)\n            elif command == 'stop':\n                break\n            elif command == 'read' and ready(value):\n                reply(value)\n    except Error as error:\n        report(error)\n        raise error\n    finally:\n        if sink is not None:\n            sink.close()\n        reply(value)\n",
        "dispatcher_break_conditional_cleanup.py",
    );
}

#[test]
fn runtime_and_reraise_handler_reaches_normal_finally() {
    for prefix in ["", "async "] {
        let source = "def f(sink, item, allow):\n    try:\n        work(item)\n    except Error as error:\n        report(error)\n        if not allow and not recoverable(error):\n            raise Error(item) from error\n        skipped(item)\n    finally:\n        sink.close()\n".replacen("def f", &format!("{prefix}def f"), 1);
        common::assert_matches_cpython_marshal(&source, "runtime_and_recovery.py");
    }
}

#[test]
fn runtime_reraise_guard_chain_reaches_normal_finally() {
    for prefix in ["", "async "] {
        let source = "def f(sink, item, suppress):\n    try:\n        result = work(item)\n    except Error as error:\n        if not recoverable(error):\n            raise\n        if not suppress:\n            raise\n        skipped(item)\n    finally:\n        if sink is not None:\n            sink.close()\n".replacen("def f", &format!("{prefix}def f"), 1);
        common::assert_matches_cpython_marshal(&source, "guard_chain_recovery.py");
    }
}

#[test]
fn async_reraise_guard_chain_reaches_optional_callback_cleanup() {
    common::assert_matches_cpython_marshal(
        "def make_worker(on_chunk_done):\n    async def run(item, suppress):\n        completed = False\n        try:\n            await work(item)\n            completed = True\n        except Error as error:\n            if not recoverable(error):\n                raise\n            if not suppress:\n                raise\n            skipped(item)\n        finally:\n            if completed and on_chunk_done is not None:\n                on_chunk_done(1)\n    return run\n",
        "async_guard_chain_optional_callback.py",
    );
}
