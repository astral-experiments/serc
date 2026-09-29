use crate::common;

#[test]
fn constant_false_guard_falls_through_to_following_statement() {
    for pattern in [
        "captured",
        "captured as alias",
        "1 | 2",
        "[captured] | (captured,)",
        "_",
    ] {
        for context in ["plain", "while", "for"] {
            let body =
                format!("match value:\n    case {pattern} if False:\n        pass\naction()\n");
            let body = match context {
                "while" => format!("while flag:\n{}", common::indent(&body)),
                "for" => format!("for value in values:\n{}", common::indent(&body)),
                _ => body,
            };
            let source = format!("def f(value, flag, values):\n{}", common::indent(&body));
            common::assert_matches_cpython_marshal(&source, "match_false_following.py");
        }
    }
    for source in [
        "def f(value):\n    match value:\n        case captured if False:\n            pass\n",
        "def f(value):\n    match value:\n        case captured if (\n            False\n        ):\n            pass\n    action()\n",
        "def f(value):\n    match value:\n        case _ if (\n            False\n        ):\n            pass\n    action()\n",
        "def f(value, flag):\n    match value:\n        case captured if flag:\n            pass\n    action()\n",
        "def f(value):\n    match value:\n        case 1 | 2 if False:\n            pass\n        case _:\n            action()\n",
        "def f(value):\n    match value:\n        case [item] as captured if False:\n            pass\n    action()\n",
        "def f(value):\n    match value:\n        case (1 | 2) as captured if False:\n            pass\n    action()\n",
    ] {
        common::assert_matches_cpython_marshal(source, "match_false_control.py");
    }
}
