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

#[test]
fn preserves_mapping_literal_key_registration_order() {
    for prefix in ["", "    'documentation'\n", "    consume('prefix')\n"] {
        for key in ["'item'", "512", "-512", "3.5", "1+2j", "b'item'"] {
            for between in ["", "    consume('between')\n"] {
                let source = format!(
                    "def f(subject):\n{prefix}    match subject:\n        case {{{key}: found, **rest}}:\n            return subject\n{between}    return {key}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "mapping_literal_key_registration_order.py",
                );
            }
        }
    }
}
