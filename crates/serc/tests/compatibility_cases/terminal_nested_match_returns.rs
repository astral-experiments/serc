use crate::common;

#[test]
fn matches_cpython_terminal_nested_match_returns() {
    let source = include_str!("../fixtures/terminal_nested_match_returns.py");
    common::assert_matches_cpython_marshal(source, "terminal_nested_match_returns.py");
}

#[test]
fn matches_cpython_terminal_false_guard_locations() {
    for guard in ["False", "0", "None", "''"] {
        let source = format!(
            "def f(kind, value):\n    match kind:\n        case 'message':\n            match value:\n                case 1:\n                    consume('first')\n                case _ if {guard}:\n                    pass\n"
        );
        common::assert_matches_cpython_marshal(&source, "terminal_false_match_guard.py");
    }
}

#[test]
fn matches_cpython_generator_boolean_sequence_marker_owners() {
    let source = "def f(kind, value):\n    match kind:\n        case 'message':\n            match value:\n                case (False, error):\n                    yield error\n                    return\n                case (True, result, recipient):\n                    consume(recipient)\n                    yield result, recipient\n";
    common::assert_matches_cpython_marshal(source, "generator_boolean_sequence_markers.py");
}
