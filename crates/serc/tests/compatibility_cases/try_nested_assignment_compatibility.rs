use crate::common;

#[test]
fn matches_cpython_marshal_for_try_nested_assignment_exits() {
    let source = r"def nested_try(first, second):
    try:
        if first:
            if second:
                value = work()
            else:
                value = other()
        else:
            value = fallback()
    except Error:
        value = recover()
    return value
";
    common::assert_matches_cpython_marshal(source, "try_nested_assignment_exit.py");
}
