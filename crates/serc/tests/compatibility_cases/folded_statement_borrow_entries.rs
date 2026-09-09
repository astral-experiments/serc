use std::fmt::Write as _;

use crate::common;

fn indent(source: &str, prefix: &str) -> String {
    let mut result = String::new();
    for line in source.lines() {
        writeln!(result, "{prefix}{line}").expect("writing to a String cannot fail");
    }
    result
}

#[test]
fn preserves_live_entries_around_folded_false_statements() {
    for (declaration, prefix, returned) in [
        ("def", "", "return value\n"),
        ("async def", "await work(other)\n", "return value\n"),
        ("def", "yield other\n", "return value\n"),
        (
            "async def",
            "yield other\nawait work(other)\n",
            "yield value\nreturn\n",
        ),
    ] {
        for condition in ["False", "0", "None"] {
            for wrapper in ["plain", "then", "else"] {
                for selected in ["result = value\n", "consume(value)\n"] {
                    let inner = format!(
                        "if {condition}:\n    consume(other)\nelse:\n{}",
                        indent(selected, "    ")
                    );
                    let branches = match wrapper {
                        "plain" => inner,
                        "then" => format!(
                            "if flag:\n{}else:\n    consume(other)\n",
                            indent(&inner, "    ")
                        ),
                        _ => format!(
                            "if flag:\n    consume(other)\nelse:\n{}",
                            indent(&inner, "    ")
                        ),
                    };
                    let body = format!(
                        "{prefix}result = value\n{branches}consume(result, value)\n{returned}"
                    );
                    let source = format!(
                        "{declaration} function(flag, value, other, consume, work):\n{}",
                        indent(&body, "    ")
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "folded_statement_live_entry.py",
                    );
                }
            }
        }
    }
}
