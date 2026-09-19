use crate::common;

#[test]
fn matches_cpython_marshal_for_folded_true_nested_if_exit() {
    let source = r"def route(value):
    if True:
        prepare()
        if value == 'left':
            first()
        elif value == 'right':
            second()
        else:
            third()
    else:
        discarded()
    if value == 'after':
        finish()
    complete()
";
    common::assert_matches_cpython_marshal(source, "folded_true_nested_if_exit.py");
}

#[test]
fn matches_cpython_marshal_for_folded_true_nested_if_without_else_control() {
    let source = r"def route(value):
    if True:
        prepare()
        if value == 'left':
            first()
        elif value == 'right':
            second()
        else:
            third()
    if value == 'after':
        finish()
    complete()
";
    common::assert_matches_cpython_marshal(source, "folded_true_nested_if_without_else_control.py");
}

#[test]
fn matches_cpython_marshal_for_folded_true_linear_exit_control() {
    let source = r"def route(value):
    if True:
        prepare()
        third()
    else:
        discarded()
    if value == 'after':
        finish()
    complete()
";
    common::assert_matches_cpython_marshal(source, "folded_true_linear_exit_control.py");
}

#[test]
fn matches_cpython_marshal_for_folded_true_terminating_final_else_control() {
    let source = r"def route(value):
    if True:
        if value == 'left':
            first()
        elif value == 'right':
            second()
        else:
            return value
    else:
        discarded()
    if value == 'after':
        finish()
    complete()
";
    common::assert_matches_cpython_marshal(source, "folded_true_terminating_final_else_control.py");
}
