use crate::common;

#[test]
fn optimized_percent_formats_preserve_surrogate_literals() {
    for expression in [
        r#""\ud800%s" % (value,)"#,
        r#""%s\udfff" % (value,)"#,
        r#""\ufffd\ud800%% %r \udfff%s tail" % (value, value)"#,
        r#""\N{REPLACEMENT CHARACTER}\ud800%s" % (value,)"#,
        r#""\ud800%10.3s\udfff" % (value,)"#,
        r#""\ud800%-10.3a\U0000dfff" % (value,)"#,
        r#""\ud800" "%s" r"\udfff" % (value,)"#,
        r#"r"\ud800%s" % (value,)"#,
        r#""\\ud800%s" % (value,)"#,
        r#""é𐀀%s�" % (value,)"#,
        r#""\ud800%%\udfff" % ()"#,
        r#""\ud800%d" % (value,)"#,
    ] {
        for source in [
            format!("def f(value):\n    return {expression}\n"),
            format!("{expression}\n"),
        ] {
            common::assert_matches_cpython_marshal(&source, "percent_format_surrogate.py");
        }
    }
}

#[test]
fn optimized_percent_formats_keep_runtime_surrogate_codepoints() {
    common::assert_executes(
        r#"value = "\ud800%s\udfff" % ("é",)
print([ord(character) for character in value])
value = "\ud800%%:%5.2s:\udfff" % ("abcd",)
print([ord(character) for character in value])
"#,
        "[55296, 233, 57343]\n[55296, 37, 58, 32, 32, 32, 97, 98, 58, 57343]\n",
    );
}
