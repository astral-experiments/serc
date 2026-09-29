use crate::common;

#[test]
fn match_recovery_preserves_pass_jump_kind_and_terminal_orphan() {
    for first in ["raise", "report(result)"] {
        for middle in ["pass", "finish()"] {
            for binding in [" as error", ""] {
                for last in ["3", "_"] {
                    let source = format!(
                        "def recover(items, mode, result):\n    output = []\n    for item in items:\n        try:\n            result = load(item)\n        except Error{binding}:\n            match mode:\n                case 1:\n                    {first}\n                case 2:\n                    {middle}\n                case {last}:\n                    report(result)\n        if result:\n            output.append(result)\n    return output\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "match_recovery_pass_orphan.py",
                    );
                }
            }
        }
    }
}
