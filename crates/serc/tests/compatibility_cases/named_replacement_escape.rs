use crate::common;

#[test]
fn named_replacement_escapes_preserve_surrogate_order() {
    for literal in [
        r#""\N{REPLACEMENT CHARACTER}\ud800""#,
        r#""\ud800\N{replacement character}\udfff""#,
        r#""\N{Replacement Character}\ufffd\ud800�\U0000dfff""#,
        r#""\\N{REPLACEMENT CHARACTER}\ud800""#,
        r#"r"\N{REPLACEMENT CHARACTER}" "\ud800""#,
        r#"f"\N{REPLACEMENT CHARACTER}\ud800{value}\udfff""#,
        r#"t"\N{REPLACEMENT CHARACTER}\ud800{value}\udfff""#,
    ] {
        common::assert_matches_cpython_marshal(
            &format!("def f(value):\n    return {literal}\n"),
            "named_replacement_escape.py",
        );
    }
}

#[test]
fn named_replacement_escapes_keep_their_runtime_codepoints() {
    common::assert_executes(
        r#"value = "\N{REPLACEMENT CHARACTER}\ud800\N{replacement character}\udfff"
print([ord(character) for character in value])
"#,
        "[65533, 55296, 65533, 57343]\n",
    );
}
