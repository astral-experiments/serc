use crate::common;

#[test]
fn matches_cpython_parenthesized_docstring_spans() {
    for header in ["", "class Example:\n", "def f():\n"] {
        for body in [
            "(\n    \"\"\"\n    doc body\n    \"\"\"\n)\nwork()\n",
            "(\n    \"first \"\n    \"second\"\n)\nwork()\n",
            "(\"body\"); pass\n",
            "(\n    \"\"\"body\"\"\"\n)\n",
        ] {
            assert_scope_matches(header, body, "parenthesized_docstring_spans.py");
        }
    }
}

#[test]
fn matches_cpython_discarded_literal_and_folded_operation_spans() {
    for header in ["", "class Example:\n", "def f():\n"] {
        for expression in [
            "\"first \"\n    \"second\"",
            "12",
            "None",
            "1, 2",
            "\"left\" +\n    \"right\"",
            "123 +\n    456",
            "-3",
            "f\"hello\"",
            "(1, 2) + (3,)",
        ] {
            for suffix in ["", "work()\n"] {
                let body = format!("work()\n(\n    {expression}\n)\n{suffix}");
                assert_scope_matches(header, &body, "discarded_constant_spans.py");
            }
        }
    }
}

fn assert_scope_matches(header: &str, body: &str, filename: &str) {
    let mut source = String::from(header);
    for line in body.lines() {
        if !header.is_empty() {
            source.push_str("    ");
        }
        source.push_str(line);
        source.push('\n');
    }
    common::assert_matches_cpython_marshal(&source, filename);
}

#[test]
fn retains_nested_class_literal_before_outer_continuation() {
    for wrapper in ["if active:", "if True:", "with manager:"] {
        for marker in ["\"unused documentation\"", "17", "1 + 2"] {
            for assignment in ["value: int = make_value()", "value = make_value()"] {
                let source = format!(
                    "class C:\n    {wrapper}\n        {assignment}\n        {marker}\n    after: int = 1\n"
                );
                common::assert_matches_cpython_marshal(&source, "class_nested_literal_marker.py");
            }
        }
    }
}
