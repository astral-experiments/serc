use crate::common;

#[test]
fn matches_cpython_fstring_escaped_newline_locations() {
    for (name, expression) in [
        ("close_brace", "f\"\"\"{x}abc}}\\\n\"\"\""),
        ("open_brace", "f\"\"\"{x}abc{{\\\n\"\"\""),
        (
            "named_unicode",
            "f\"\"\"{x}\\N{RIGHT CURLY BRACKET}\\\n\"\"\"",
        ),
        ("literal_only", "f\"\"\"abc}}\\\n\"\"\""),
        (
            "named_format_spec",
            "f\"\"\"{x:\\N{RIGHT CURLY BRACKET}\\\n}\"\"\"",
        ),
        ("repeated_newline", "f\"\"\"{x}abc}}\\\n\\\n\"\"\""),
        ("next_interpolation", "f\"\"\"{x}abc}}\\\n{y}\"\"\""),
        ("utf8_prefix", "f\"\"\"{x}é中}}\\\n\"\"\""),
        ("unpaired_brace_escape", "f\"\"\"{x}\\{{\\\n\"\"\""),
        ("plain_control", "f\"\"\"{x}abc\\\n\"\"\""),
        ("raw_control", "fr\"\"\"{x}abc}}\\\n\"\"\""),
        ("unicode_hex_control", "f\"\"\"{x}\\u007d\\\n\"\"\""),
        ("suffix_control", "f\"\"\"{x}abc}}\\\nz\"\"\""),
        ("ordinary_newline_control", "f\"\"\"{x}abc}}\n\"\"\""),
        ("escaped_backslash_control", "f\"\"\"{x}abc}}\\\\\n\"\"\""),
        (
            "concatenation_control",
            "f\"\"\"{x}abc}}\\\n\"\"\" \"tail\"",
        ),
    ] {
        for operation in ["return ", ""] {
            let source = format!("def f(x, y):\n    {operation}{expression}\n");
            common::assert_matches_cpython_marshal(&source, &format!("{name}.py"));
        }
    }
}

#[test]
fn matches_cpython_fstring_escaped_newline_line_endings() {
    let expression = "f\"\"\"{x}abc}}\\\n\"\"\"";
    for newline in ["\n", "\r\n", "\r"] {
        let source = format!("def f(x, y):\n    return {expression}\n").replace('\n', newline);
        common::assert_matches_cpython_marshal(&source, "fstring_line_ending.py");
    }
}
