use crate::common;

#[test]
fn matches_cpython_class_pattern_wildcard_locations() {
    for pattern in [
        "Record(0, _)",
        "Record(kind=0, ignored=_)",
        "Container(Record(kind=0, ignored=_), value)",
        "Container(record=Record(kind=0, ignored=_), value=value)",
    ] {
        let source = format!(
            "def f(subject):\n    match subject:\n        case {pattern}:\n            return True\n    return False\n"
        );
        common::assert_matches_cpython_marshal(&source, "class_pattern_wildcard_locations.py");
    }
}
