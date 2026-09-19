use crate::common;

#[test]
fn matches_cpython_marshal_for_nested_if_match_loop() {
    let source = r"def f(items, outer, inner, alternate):
    for item in items:
        match item:
            case 1:
                if outer:
                    if inner:
                        first()
                elif alternate:
                    second()
            case _:
                third()
";
    common::assert_matches_cpython_marshal(source, "nested_if_match_loop.py");
}

#[test]
fn matches_cpython_marshal_for_nested_if_terminating_else_control() {
    let source = r"def f(outer, inner):
    if outer:
        if inner:
            first()
    else:
        return
    finish()
";
    common::assert_matches_cpython_marshal(source, "nested_if_terminating_else_control.py");
}

#[test]
fn nested_final_else_cleanup_keeps_condition_exit_location() {
    let source = r#"def render(mode, enabled, first, second, prepare, write, report):
    result = None
    if mode:
        if enabled():
            if not first:
                result = "first missing"
            elif not second:
                result = "second missing"
            else:
                path = prepare()
                if not path:
                    result = "path missing"
                else:
                    try:
                        result = write(path)
                    except Exception as error:
                        result = str(error)
    else:
        report("other mode")
    report(result)
"#;
    common::assert_matches_cpython_marshal(source, "nested_cleanup_if_exit_donor.py");
}

#[test]
fn nested_final_else_linear_work_keeps_shared_exit_location() {
    let source = r#"def render(mode, enabled, first, second, prepare, write, report):
    result = None
    if mode:
        if enabled():
            if not first:
                result = "first missing"
            elif not second:
                result = "second missing"
            else:
                path = prepare()
                if not path:
                    result = "path missing"
                else:
                    result = write(path)
    else:
        report("other mode")
    report(result)
"#;
    common::assert_matches_cpython_marshal(source, "nested_linear_if_exit_control.py");
}

#[test]
fn nested_final_else_pass_keeps_shared_exit_location() {
    let source = r#"def render(mode, enabled, first, second, prepare, write, report):
    result = None
    if mode:
        if enabled():
            if not first:
                result = "first missing"
            elif not second:
                result = "second missing"
            else:
                path = prepare()
                if not path:
                    result = "path missing"
                else:
                    try:
                        result = write(path)
                    except Exception as error:
                        result = str(error)
                    pass
    else:
        report("other mode")
    report(result)
"#;
    common::assert_matches_cpython_marshal(source, "nested_cleanup_if_exit_pass_control.py");
}

#[test]
fn nested_final_else_manager_keeps_condition_exit_location() {
    let source = r#"def render(mode, enabled, first, second, prepare, write, report):
    result = None
    if mode:
        if enabled():
            if not first:
                result = "first missing"
            elif not second:
                result = "second missing"
            else:
                path = prepare()
                if not path:
                    result = "path missing"
                else:
                    with write(path) as value:
                        result = value
    else:
        report("other mode")
    report(result)
"#;
    common::assert_matches_cpython_marshal(source, "nested_cleanup_if_exit_with_boundary.py");
}

#[test]
fn nested_final_else_exception_group_keeps_exit_location() {
    let source = r#"def render(mode, enabled, first, second, prepare, write, report):
    result = None
    if mode:
        if enabled():
            if not first:
                result = "first missing"
            elif not second:
                result = "second missing"
            else:
                path = prepare()
                if not path:
                    result = "path missing"
                else:
                    try:
                        result = write(path)
                    except* Exception as error:
                        result = str(error)
    else:
        report("other mode")
    report(result)
"#;
    common::assert_matches_cpython_marshal(
        source,
        "nested_cleanup_if_exit_exception_group_boundary.py",
    );
}
