use crate::common;

#[test]
fn matches_cpython_marshal_for_discarded_suspension_constants() {
    for suspension in [
        "yield",
        "yield from values",
        "yield from produce('operand', key='keyword')",
        "yield 'operand'",
        "yield None",
    ] {
        for prefix in ["", "    'docstring'\n", "    consume(None)\n"] {
            for discarded in [
                format!("    if flag:\n        raise Error\n        {suspension}\n"),
                format!("    if flag:\n        raise Error('early')\n        {suspension}\n"),
                format!("    if False:\n        {suspension}\n"),
                format!("    while False:\n        {suspension}\n"),
            ] {
                let source = format!(
                    "def f(flag):\n{prefix}{discarded}    consume('later')\n    yield from values\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "discarded_suspension_constants.py",
                );
            }
        }
    }
}
