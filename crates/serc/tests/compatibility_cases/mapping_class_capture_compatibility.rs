use crate::common;

#[test]
fn matches_cpython_marshal_for_mapping_class_captures() {
    let source = r#"def mapping_class(subject):
    match subject:
        case {"x": str() as value}:
            return value
        case _:
            return None


def mapping_class_guard(subject):
    match subject:
        case {"x": list() as values} if values:
            return values
        case _:
            return None


def nested_mapping_class(subject):
    match subject:
        case {"name": str() as name}:
            return name
        case {"id": int() as identifier}:
            return identifier
        case _:
            return None
"#;
    common::assert_matches_cpython_marshal(source, "mapping_class_captures.py");
}

#[test]
fn matches_cpython_marshal_for_pending_mapping_class_captures() {
    for (filename, source) in [
        (
            "pending_mapping_captures_class_as_control.py",
            r#"def f(value):
    match value:
        case {
            "pool": {
                "created": created,
                "name": name,
                "kind": kind,
                "label": label,
            },
            "pod": {"ready": bool() as ready},
        }:
            return created, name, kind, label, ready
    return None
"#,
        ),
        (
            "same_line_pending_mapping_class_capture_control.py",
            r#"def f(value):
    match value:
        case {"pool": {"created": created, "name": name, "kind": kind, "label": label}, "pod": {"ready": bool() as ready}}:
            return created, name, kind, label, ready
    return None
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
