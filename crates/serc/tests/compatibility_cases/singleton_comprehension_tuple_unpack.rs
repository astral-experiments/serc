use crate::common;

#[test]
fn matches_cpython_marshal_for_singleton_comprehension_tuple_unpack() {
    for kind in ["list", "set", "dict", "generator"] {
        for (target, value, element) in [
            ("a,", "(True,)", "a"),
            ("a, b", "(True, False)", "(a, b)"),
            ("a, b, c", "(1, 2, 3)", "(a, b, c)"),
            ("a, b, c, d", "(1, 2, 3, 4)", "(a, b, c, d)"),
            ("[a, b]", "(f(1), f(2))", "(a, b)"),
            ("a,", "(f(1) if flag else f(2),)", "a"),
            ("a, (b, c)", "(1, (2, 3))", "(a, b, c)"),
            ("((a, b),)", "((1, 2),)", "(a, b)"),
            ("a, *b", "(1, 2)", "(a, tuple(b))"),
            ("a, b", "(1, *items)", "(a, b)"),
            ("a, b", "[True, False]", "(a, b)"),
            ("obj.a, obj.b", "(f(1), f(2))", "x"),
            ("obj[0], obj[1]", "(f(1), f(2))", "x"),
            (
                "(\n            a,\n        )",
                "(\n            True,\n        )",
                "a",
            ),
            (
                "(\n            a,\n            b\n        )",
                "(\n            True,\n            False\n        )",
                "(a, b)",
            ),
            (
                "((obj\n            if flag else other).attr,)",
                "(f(x),)",
                "x",
            ),
        ] {
            for suffix in ["", " if flag for y in items"] {
                let clauses = format!("for x in xs for {target} in [{value}]{suffix}");
                let expression = match kind {
                    "list" => format!("[{element} {clauses}]"),
                    "set" => format!("{{{element} {clauses}}}"),
                    "dict" => format!("{{{element}: x {clauses}}}"),
                    "generator" => format!("({element} {clauses})"),
                    _ => unreachable!(),
                };
                let source = format!(
                    "def subject(xs, items, f, flag, obj, other):\n    return {expression}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "singleton_comprehension_tuple_unpack.py",
                );
            }
        }
    }
}
