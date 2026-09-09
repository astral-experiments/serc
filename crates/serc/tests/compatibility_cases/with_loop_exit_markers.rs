use std::fmt::Write;

use crate::common;

#[test]
fn matches_cpython_with_loop_exit_markers() {
    for header in ["for item in values:", "while predicate(values):"] {
        for body in [
            "break",
            "if predicate(item):\n    break",
            "try:\n    consume(item)\n    break\nexcept Error:\n    consume(item)",
            "if first(item):\n    break\nif second(item):\n    break",
            "consume(item)",
        ] {
            for alternate in ["raise Error", "return item", "consume(item)", ""] {
                let mut loop_body = format!("{header}\n    {}", body.replace('\n', "\n    "));
                if !alternate.is_empty() {
                    write!(loop_body, "\nelse:\n    {alternate}").unwrap();
                }
                let source = format!(
                    "def f(values):\n    with manager():\n        {}\n    consume(values)\n",
                    loop_body.replace('\n', "\n        "),
                );
                common::assert_matches_cpython_marshal(&source, "with_loop_exit_markers.py");
            }
        }
    }
}
