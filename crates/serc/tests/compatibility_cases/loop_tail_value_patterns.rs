use std::fmt::Write;

use crate::common;

#[test]
fn matches_cpython_final_value_pattern_loop_failures() {
    let indent = |source: &str| {
        let mut indented = String::new();
        for line in source.lines() {
            writeln!(indented, "    {line}").unwrap();
        }
        indented
    };
    for mode in ["sync", "async", "generator"] {
        for header in ["for value in items:", "while ready():"] {
            for pattern in ["\"second\"", "Kinds.SECOND"] {
                let body = format!(
                    "{header}\n    match value:\n        case \"first\":\n            first(value)\n        case {pattern}:\n            report(value)\n"
                );
                let indented = indent(&body);
                for suite in [
                    body.clone(),
                    format!("try:\n{indented}except Exception:\n    recover()\n"),
                    format!("with manager():\n{indented}"),
                ] {
                    let definition = if mode == "async" { "async def" } else { "def" };
                    let suspension = if mode == "generator" {
                        "    yield None\n"
                    } else {
                        ""
                    };
                    let suite = indent(&suite);
                    let source = format!(
                        "{definition} f(items, value):\n{suspension}{suite}    return None\n"
                    );
                    common::assert_matches_cpython_marshal(&source, "loop_tail_value_patterns.py");
                }
            }
        }
    }
}
