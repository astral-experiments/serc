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
