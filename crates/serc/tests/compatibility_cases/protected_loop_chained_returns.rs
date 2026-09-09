use crate::common;

#[test]
fn matches_cpython_protected_loop_chained_return_copies() {
    for header in ["while True:", "while ready():"] {
        for prefix in [
            "",
            "            choice = fetch()\n            if not choice:\n                continue\n            value = convert(choice)\n",
        ] {
            let source = format!(
                "def f(value, limit):\n    {header}\n        try:\n{prefix}            if 0 <= value < limit:\n                return value\n        except Exception:\n            pass\n    finish()\n"
            );
            common::assert_matches_cpython_marshal(&source, "protected_loop_chained_returns.py");
        }
    }
}
