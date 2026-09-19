use crate::common;

#[test]
fn matches_cpython_borrows_after_protected_constant_if_else() {
    let body =
        "if True:\n    saved = value\nelse:\n    consume(other)\nconsume(value)\nconsume(other)";
    for prefix in ["", "async "] {
        for (opening, closing) in [
            ("with manager:", ""),
            ("try:", "\nexcept Error:\n    recover()"),
            ("try:", "\nfinally:\n    consume(other)"),
            (
                "try:",
                "\nexcept Error:\n    recover()\nfinally:\n    consume(other)",
            ),
        ] {
            let protected = format!("{opening}\n    {}{closing}", body.replace('\n', "\n    "));
            let source = format!(
                "{prefix}def guarded(manager, value, other):\n    {}\n",
                protected.replace('\n', "\n    "),
            );
            common::assert_matches_cpython_marshal(&source, "protected_constant_if_else.py");
        }
    }
}

#[test]
fn matches_cpython_borrow_bytes_after_empty_finally_in_manager() {
    // These empty cleanup paths have a separate, pre-existing exception-table mismatch.
    // Protect their matching instruction and location bytes against the pinned oracle.
    const ORACLE: &str = r#"
import marshal, pathlib, sys, types
ours = marshal.loads(pathlib.Path(sys.argv[1]).read_bytes())
reference = compile(pathlib.Path(sys.argv[2]).read_text(), "empty_finally_manager_borrows.py", "exec", dont_inherit=True, optimize=0)
ours = next(value for value in ours.co_consts if isinstance(value, types.CodeType))
reference = next(value for value in reference.co_consts if isinstance(value, types.CodeType))
assert ours.co_code == reference.co_code, "instruction bytes differ"
assert ours.co_linetable == reference.co_linetable, "location table bytes differ"
"#;
    for (prefix, receive, work) in [
        ("", "receive(resource)", "consume(value)"),
        ("async ", "await receive(resource)", "consume(value)"),
        ("", "receive(resource)", "yield value"),
    ] {
        for cleanup in ["pass", "42"] {
            let source = format!(
                "{prefix}def guarded(resource, receive, consume, manager):\n    with manager:\n        try:\n            while True:\n                value = {receive}\n                if value is None:\n                    break\n                {work}\n        finally:\n            {cleanup}\n    consume(resource)\n",
            );
            let directory = tempfile::tempdir().unwrap();
            let source_path = directory.path().join("source.py");
            let marshal_path = directory.path().join("ours.marshal");
            std::fs::write(&source_path, &source).unwrap();
            std::fs::write(
                &marshal_path,
                serc::compile(&source, "empty_finally_manager_borrows.py")
                    .unwrap()
                    .marshal(),
            )
            .unwrap();
            let output = common::python_3145_command()
                .args(["-c", ORACLE])
                .arg(&marshal_path)
                .arg(&source_path)
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
fn matches_cpython_borrows_through_surviving_try_exit_jumps() {
    for prefix in ["", "async "] {
        for handler in [
            "except Error:\n    raise ValueError()",
            "except Error as error:\n    raise ValueError() from error",
            "except Error:\n    recover()",
        ] {
            for middle in ["", "    elif other:\n        consume(value)\n"] {
                let branch = format!("try:\n    consume(value)\n{handler}");
                let branch = branch.replace('\n', "\n        ");
                let source = format!(
                    "{prefix}def guarded(flag, value, other):\n    if flag:\n        {branch}\n{middle}    else:\n        {branch}\n    if value is not None:\n        consume(value)\n    return other\n"
                );
                common::assert_matches_cpython_marshal(&source, "surviving_try_exit_jumps.py");
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_borrow_fusion_across_copied_exits() {
    let source = r#"def ternary(a, b, c):
    return a if a else None, b, c

def pair(a, b):
    return a if a else None, b

def leading(a, b, c):
    return 1, a if a else None, b, c

def both_names(flag, a, b, c):
    return a if flag else a, b, c

def called(a, b, c):
    return consume(a if a else None, b, c)

def guarded(data):
    error_message = ""
    name = ""
    order = None
    if not data:
        error_message += "missing data"
    else:
        try:
            name = str(data["name"])
        except KeyError:
            error_message += "name has to be defined"
        try:
            order = int(str(data["order"]))
        except KeyError:
            error_message += "order has to be defined"
        except ValueError:
            error_message += "order has to be a number"
    return error_message if error_message else None, name, order

def quartet(a, b, c, d):
    return a if a else None, b, c, d

def inverted(a, b, c):
    return None if a else a, b, c
"#;
    common::assert_matches_cpython_marshal(source, "copied_exit_borrow_fusion_compatibility.py");
}

#[test]
fn matches_cpython_borrows_across_pre_fold_exit_size_threshold() {
    for expression in [
        "value",
        "value.name",
        "value + 2",
        "1 + 2",
        "value + (1 + 1)",
        "(1, 2)",
        "(value, 1)",
        "lambda: value",
        "value in [1]",
        "value in [1, 2, 3]",
        "value in {1}",
        "value in {1, 2}",
        "value in {1, 2, 3}",
        "value in (1,)",
        "value in (1, 2, 3)",
    ] {
        let source = format!(
            "def guarded(value):\n    try:\n        call()\n    except Error:\n        return False\n    return {expression}\n"
        );
        common::assert_matches_cpython_marshal(&source, "pre_fold_exit_size.py");
    }
}

#[test]
fn matches_cpython_borrows_after_mixed_terminating_handlers() {
    for handlers in [
        "except Error:\n    return None\nexcept Other:\n    recover()",
        "except Error:\n    recover()\nexcept Other:\n    return None",
        "except Error:\n    raise\nexcept Other:\n    pass",
        "except Error:\n    pass\nexcept Other:\n    raise",
    ] {
        for tail in [
            "return value",
            "return value.name",
            "return value in [1, 2, 3]",
            "result = value.attr()\nreturn consume(result, value)",
        ] {
            let source = format!(
                "def guarded(value):\n    try:\n        call(value)\n    {}\n    {}\n",
                handlers.replace('\n', "\n    "),
                tail.replace('\n', "\n    "),
            );
            common::assert_matches_cpython_marshal(&source, "mixed_terminating_handlers.py");
        }
    }
}

#[test]
fn matches_cpython_borrows_after_finally_condition_joins() {
    for (prefix, body) in [
        ("", "call(value)"),
        ("", "yield value"),
        ("async ", "await call(value)"),
    ] {
        for condition in ["value", "value.ready", "value is not None", "value == 1"] {
            for tail in [
                "return value",
                "return value, 1, 2",
                "result = value.attr()\nreturn consume(result, value)",
            ] {
                let source = format!(
                    "{prefix}def guarded(value):\n    try:\n        {body}\n    finally:\n        if {condition}:\n            finish()\n    {}\n",
                    tail.replace('\n', "\n    "),
                );
                common::assert_matches_cpython_marshal(&source, "finally_condition_joins.py");
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_after_constant_if_elimination() {
    for condition in ["True", "False", "1", "0"] {
        for body in ["call(value)", "result = value.attr()", "pass"] {
            for tail in [
                "return value",
                "result = value.attr()\nreturn consume(result, value)",
            ] {
                let source = format!(
                    "def guarded(value):\n    if {condition}:\n        {body}\n    else:\n        fallback(value)\n    {}\n",
                    tail.replace('\n', "\n    "),
                );
                common::assert_matches_cpython_marshal(&source, "constant_if_borrows.py");
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_after_context_manager_loop_breaks() {
    for prefix in ["", "async "] {
        for body in [
            "item = get()\nif not item:\n    break\nwork(item)",
            "item = get()\nif not item:\n    break\nelse:\n    work(item)",
            "work(value)\nbreak",
        ] {
            for tail in [
                "return value",
                "return value.method()",
                "result = value.attr()\nreturn use(result, value)",
            ] {
                let source = format!(
                    "{prefix}def guarded(context, value):\n    {prefix}with context:\n        while True:\n            {}\n    {}\n",
                    body.replace('\n', "\n            "),
                    tail.replace('\n', "\n    "),
                );
                common::assert_matches_cpython_marshal(&source, "manager_loop_break_borrows.py");
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_through_conditional_break_finally_edges() {
    for (prefix, receive, work) in [
        ("", "receive(resource)", "consume(value)"),
        ("async ", "await receive(resource)", "consume(value)"),
        ("", "receive(resource)", "yield value"),
    ] {
        for tail in [
            format!("if value is None:\n    break\n{work}"),
            format!("if value is None:\n    break\nelse:\n    {work}"),
            format!("{work}\nif value is None:\n    break"),
            format!("{work}\nbreak"),
        ] {
            let loop_body = format!("value = {receive}\n{tail}");
            let loop_suite = format!("while True:\n    {}", loop_body.replace('\n', "\n    "));
            for protected in [
                loop_suite.clone(),
                format!("{loop_suite}\nconsume(resource)"),
                format!(
                    "try:\n    {}\nfinally:\n    consume(resource)",
                    loop_suite.replace('\n', "\n    "),
                ),
            ] {
                let source = format!(
                    "{prefix}def guarded(resource, receive, consume):\n    try:\n        {}\n    finally:\n        consume(resource)\n    consume(resource)\n",
                    protected.replace('\n', "\n        "),
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "conditional_break_finally_edges.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_after_conditional_breaks_in_nested_handlers() {
    for (prefix, receive, work) in [
        ("", "receive(resource)", "consume(value)"),
        ("async ", "await receive(resource)", "consume(value)"),
        ("", "receive(resource)", "yield value"),
    ] {
        for tail in [
            format!("if value is None:\n    break\n{work}"),
            format!("if value is None:\n    break\nelse:\n    {work}"),
        ] {
            let source = format!(
                "{prefix}def guarded(resource, receive, consume):\n    try:\n        try:\n            while True:\n                value = {receive}\n                {}\n        except Exception:\n            consume(resource)\n    finally:\n        consume(resource)\n    consume(resource)\n",
                tail.replace('\n', "\n                "),
            );
            common::assert_matches_cpython_marshal(&source, "conditional_break_nested_handlers.py");
        }
    }
}

#[test]
fn matches_cpython_borrows_through_manager_finally_calls() {
    for (prefix, receive, suspension) in [
        ("", "receive(resource)", ""),
        ("async ", "await receive(resource)", ""),
        ("", "receive(resource)", "    yield resource\n"),
    ] {
        for body in [
            format!("result = {receive}"),
            format!("if flag:\n    result = {receive}\nelse:\n    consume(resource)"),
            format!("try:\n    result = {receive}\nexcept Exception:\n    consume(resource)"),
        ] {
            for cleanup in [
                "consume(resource)",
                "consume(resource)\nconsume(result)",
                "resource",
            ] {
                for managers in ["manager", "manager, other"] {
                    let source = format!(
                        "{prefix}def guarded(manager, other, resource, receive, consume, flag, result):\n{suspension}    with {managers}:\n        try:\n            {}\n        finally:\n            {}\n    consume(result)\n",
                        body.replace('\n', "\n            "),
                        cleanup.replace('\n', "\n            "),
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "manager_finally_borrow_calls.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_after_manager_finally_constant_assertions() {
    // Constant-true assertions have separate existing cleanup-load and exception-table differences.
    // Compare the following statement to the pinned oracle to protect its borrow boundary.
    const ORACLE: &str = r#"
import dis, marshal, pathlib, sys, types
source = pathlib.Path(sys.argv[2]).read_text()
ours = marshal.loads(pathlib.Path(sys.argv[1]).read_bytes())
reference = compile(source, "manager_finally_empty_body.py", "exec", dont_inherit=True, optimize=0)
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
    for (prefix, suspension) in [("", ""), ("async ", ""), ("", "    yield resource\n")] {
        for managers in ["manager", "manager, other"] {
            let source = format!(
                "{prefix}def guarded(manager, other, resource, consume, result):\n{suspension}    with {managers}:\n        try:\n            assert True\n        finally:\n            consume(resource)\n    consume(result)\n",
            );
            let directory = tempfile::tempdir().unwrap();
            let source_path = directory.path().join("source.py");
            let marshal_path = directory.path().join("ours.marshal");
            std::fs::write(&source_path, &source).unwrap();
            std::fs::write(
                &marshal_path,
                serc::compile(&source, "manager_finally_empty_body.py")
                    .unwrap()
                    .marshal(),
            )
            .unwrap();
            let output = common::python_3145_command()
                .args(["-c", ORACLE])
                .arg(&marshal_path)
                .arg(&source_path)
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
fn matches_cpython_borrows_through_finally_protected_manager_breaks() {
    for (prefix, receive, suspension) in [
        ("", "receive()", ""),
        ("async ", "await receive()", ""),
        ("", "receive()", "    yield result\n"),
    ] {
        for managers in ["manager", "manager, other"] {
            for condition in ["item is None", "not item"] {
                for tail in [
                    format!("if {condition}:\n    break\nconsume(item)"),
                    format!("if {condition}:\n    break\nelse:\n    consume(item)"),
                    format!("consume(item)\nif {condition}:\n    break"),
                ] {
                    let source = format!(
                        "{prefix}def guarded(manager, other, receive, consume, finish, result):\n{suspension}    try:\n        with {managers}:\n            while True:\n                item = {receive}\n                {}\n    finally:\n        finish()\n    return consume(result)\n",
                        tail.replace('\n', "\n                "),
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "finally_protected_manager_breaks.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_at_finally_while_headers_after_breaks() {
    for (prefix, work) in [
        ("", "consume(resource)"),
        ("async ", "await consume(resource)"),
        ("", "yield resource"),
    ] {
        for body in [
            format!("if flag:\n    break\n{work}"),
            format!("if flag:\n    if other:\n        break\n    continue\n{work}"),
            format!("if flag:\n    break\nfor item in items:\n    {work}"),
        ] {
            for cleanup in [
                "while other:\n    consume(resource)",
                "while other:\n    for item in items:\n        consume(resource)",
            ] {
                let source = format!(
                    "{prefix}def guarded(flag, other, items, consume, resource):\n    try:\n        while True:\n            {}\n    finally:\n        {}\n",
                    body.replace('\n', "\n            "),
                    cleanup.replace('\n', "\n        "),
                );
                common::assert_matches_cpython_marshal(&source, "break_finally_while.py");
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_at_truthy_finally_while_headers_after_breaks() {
    for (prefix, leading) in [("", ""), ("async ", ""), ("", "    yield resource\n")] {
        for condition in ["True", "1"] {
            for body in [
                "consume(resource)",
                "for item in items:\n    consume(item, resource)",
            ] {
                for entry in ["", "        pass\n"] {
                    let source = format!(
                        "{prefix}def guarded(resource, flag, items):\n{leading}    try:\n        while True:\n            if flag:\n                break\n            consume(resource)\n    finally:\n{entry}        while {condition}:\n            {}\n",
                        body.replace('\n', "\n            "),
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "break_truthy_finally_while.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_local_fusion_at_copied_keyword_call_exits() {
    let source = r"def conditional_keyword(flag, value, other):
    return consume(value if flag else None, extra=other)

def inverted_keyword(flag, value, other):
    return consume(None if flag else value, extra=other)

def multiline_keyword(flag, value, other):
    return consume(value if flag else None,
                   extra=other)
";
    common::assert_matches_cpython_marshal(source, "copied_keyword_local_fusion.py");
}

#[test]
fn matches_cpython_local_fusion_at_copied_attribute_assignment_exits() {
    let source = r"class Config:
    def __init__(self, value):
        self.value = value if value is not None else []

    def replace(self, flag, value):
        self.value = value if flag else None

    def replace_and_return(self, flag, value):
        self.value = value if flag else None
        return value
";
    common::assert_matches_cpython_marshal(source, "copied_attribute_local_fusion.py");
}

#[test]
fn matches_cpython_local_fusion_at_copied_operator_exits() {
    let source = r#"def concatenate(flag, value, other):
    return (value if flag else "") + other

def identical(flag, value, other):
    return (value if flag else None) is other

def positional(flag, value, other):
    return consume(value if flag else None, other)
"#;
    common::assert_matches_cpython_marshal(source, "copied_operator_local_fusion.py");
}

#[test]
fn matches_cpython_local_fusion_at_keyword_return_seams_after_comprehensions() {
    common::assert_matches_cpython_marshal(
        r"async def f(values, selected, tail):
    items = [item for item in values]
    candidate = await fetch(items)
    return pack(items=items, chosen=candidate if selected else None, tail=tail)
",
        "async_comprehension_conditional_keyword_tail.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(values, candidate, selected, tail):
    items = [item.copy() for item in values]
    return pack(items, chosen=candidate if selected else None, tail=tail)
",
        "comprehension_conditional_keyword_tail.py",
    );
}

#[test]
fn matches_cpython_marshal_for_protected_conditional_positional_return_fusion_target() {
    let source = r"import service

def f(text, *, direct, sensitive):
    if not text:
        raise ValueError('text is required')
    options = 0 if sensitive else service.DEFAULT
    try:
        return service.compile(text if direct else service.escape(text), options)
    except service.Error as error:
        raise ValueError(f'invalid pattern: {error}') from error
";
    common::assert_matches_cpython_marshal(
        source,
        "protected_conditional_positional_return_fusion_target.py",
    );
}

#[test]
fn matches_cpython_marshal_for_unprotected_conditional_positional_return_fusion_control() {
    let source = r"import service

def f(text, *, direct, sensitive):
    if not text:
        raise ValueError('text is required')
    options = 0 if sensitive else service.DEFAULT
    return service.compile(text if direct else service.escape(text), options)
";
    common::assert_matches_cpython_marshal(
        source,
        "unprotected_conditional_positional_return_fusion_control.py",
    );
}

#[test]
fn matches_cpython_marshal_for_protected_conditional_tuple_return_fusion_target() {
    let source = r"def f(view, path, extra):
    try:
        return view, path if exists(path) else None, extra
    except Error as error:
        record(error)
        return None, None, None
";
    common::assert_matches_cpython_marshal(
        source,
        "protected_conditional_tuple_return_fusion_target.py",
    );
}

#[test]
fn matches_cpython_marshal_for_unprotected_conditional_tuple_return_fusion_control() {
    let source = r"def f(view, path, extra):
    return view, path if exists(path) else None, extra
";
    common::assert_matches_cpython_marshal(
        source,
        "unprotected_conditional_tuple_return_fusion_control.py",
    );
}
