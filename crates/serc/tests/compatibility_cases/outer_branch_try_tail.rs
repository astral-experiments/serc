use crate::common;

#[test]
fn outer_branch_try_tail_target() {
    let source = r#"def normalize(value):
    if isinstance(value, str):
        text = value.strip()
        if text.lower() in {"", "none"}:
            return None
        try:
            if int(text) < 0:
                return None
        except ValueError:
            pass
    elif type(value) is int and value < 0:
        return None
    return value
"#;
    common::assert_matches_cpython_marshal(source, "outer_branch_try_tail_target.py");
}

#[test]
fn outer_branch_try_tail_call_control() {
    let source = r#"def normalize(value):
    if isinstance(value, str):
        text = value.strip()
        if text.lower() in {"", "none"}:
            return None
        try:
            if int(text) < 0:
                return None
            body_finished()
        except ValueError:
            pass
    elif type(value) is int and value < 0:
        return None
    return value
"#;
    common::assert_matches_cpython_marshal(source, "outer_branch_try_tail_call_control.py");
}
