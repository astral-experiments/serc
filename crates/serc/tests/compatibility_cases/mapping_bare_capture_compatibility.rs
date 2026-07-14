use crate::common;

#[test]
fn matches_cpython_marshal_for_guarded_final_mapping_capture() {
    let source = r#"def guarded_final(subject, use):
    match subject:
        case {"skip": True}:
            return []
        case {"key": value} if use(value):
            return value
        case _:
            return []

def guarded_single(subject, use):
    match subject:
        case {"key": value} if use(value):
            return value
        case _:
            return []
"#;
    common::assert_matches_cpython_marshal(source, "mapping_bare_capture_compatibility.py");
}
