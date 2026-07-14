use crate::common;

#[test]
fn matches_cpython_marshal_for_copied_finally_none_comparisons() {
    let cases = [
        (
            "name_condition_control.py",
            r"
def name_condition_control(resource):
    try:
        return produce()
    finally:
        if resource:
            resource.close()
",
        ),
        (
            "following_return_control.py",
            r"
def following_return_control(resource):
    try:
        result = produce()
    finally:
        if resource is not None:
            resource.close()
    return result
",
        ),
        (
            "call_positive.py",
            r"
def call_positive(resource):
    try:
        return produce()
    finally:
        if resource is not None:
            resource.close()
",
        ),
        (
            "name_return_positive.py",
            r"
def name_return_positive(resource, result):
    try:
        return result
    finally:
        if resource is not None:
            resource.close()
",
        ),
    ];

    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
