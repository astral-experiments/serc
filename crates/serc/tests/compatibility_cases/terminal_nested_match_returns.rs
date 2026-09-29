use crate::common;

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
fn terminal_match_break_retains_native_first_constant() {
    for pattern in [
        "{\"key\": captured}",
        "{\"key\": captured, \"other\": other}",
        "[captured]",
        "[first, second]",
        "Point(captured)",
        "Point(field=captured)",
        "1 | 2",
    ] {
        for prefix in ["", "42\n", "initial = 42\n"] {
            let body =
                format!("{prefix}match value:\n    case {pattern}:\n        break\naction()\n");
            let source = format!(
                "def f(value, flag):\n    while flag:\n{}",
                common::indent(&common::indent(&body))
            );
            common::assert_matches_cpython_marshal(&source, "match_loop_first_constant.py");
        }
    }
    for source in [
        "def f(value, flag):\n    while flag:\n        42\n        break\n",
        "def f(value, flag):\n    while flag:\n        match value:\n            case {\"key\": captured}:\n                break\n        action()\n    return 3\n",
    ] {
        common::assert_matches_cpython_marshal(source, "match_loop_first_constant_control.py");
    }
}
