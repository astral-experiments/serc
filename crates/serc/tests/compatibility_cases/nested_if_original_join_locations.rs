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
