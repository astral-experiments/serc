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

#[test]
fn folded_populated_predecessors_keep_original_tuple_copy_rounds() {
    for value in [
        "1",
        "-1",
        "(1, 2, 3)",
        "values[None, :]",
        "prepare(values, (0, 1), mode='constant', constant_values=0)",
    ] {
        for returned in ["left, right", "left, right, values"] {
            let source = format!(
                "def choose(outer, first, second, left, right, values):
    if outer:
        if first:
            left = {value}
        elif second:
            if left:
                right = False
            else:
                right = True
        else:
            raise ValueError
    else:
        left = 4
    return {returned}
"
            );
            common::assert_matches_cpython_marshal(&source, "folded_predecessor_tuple_copy.py");
        }
    }
}
