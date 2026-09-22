use crate::common;

#[test]
fn matches_cpython_surrogate_string_constant_folding() {
    for expression in [
        r#""\ud800" + "tail""#,
        r#""head" + "\udfff""#,
        r#""\ud800" + "\U0000dfff""#,
        r#"("head" + "\ud800") + "tail""#,
        r#"("\ud800" + "\udfff") * 2"#,
        r#"2 * ("head" + "\ud800")"#,
        r#"("\ud800" + "tail", "head" + "\udfff")"#,
        r#"r"\ud800" "�""#,
        r#"r"\U0000dfff" "\ufffd""#,
        r#""�" + "\ud800""#,
        r#""\ud800" + "�""#,
        r#"r"\ud800" "�" + "\udfff""#,
    ] {
        for source in [
            format!("value = {expression}\n"),
            format!("def f():\n    return {expression}\n"),
        ] {
            common::assert_matches_cpython_marshal_bytes(
                source.as_bytes(),
                "surrogate_string_folding.py",
            );
        }
    }
}

#[test]
fn preserves_tuple_truth_runtime_steps_during_surrogate_folding() {
    for expression in [
        r"not ()",
        r#"not ("\ud800",)"#,
        r#"((not ("\ud800",)), "\udfff")"#,
        r"(not (),) * 2",
        r#"(not (), "\ud800")[0]"#,
        r"(17, 23)[not ()]",
        r#"(not ("\ud800",)) + 1"#,
        r#"(not (),) + ("\ud800",)"#,
        r#"not (runtime(), "\ud800")"#,
    ] {
        for source in [
            format!("value = {expression}\n"),
            format!("def f():\n    return {expression}\n"),
        ] {
            common::assert_matches_cpython_marshal_bytes(
                source.as_bytes(),
                "tuple_truth_surrogate_folding.py",
            );
        }
    }
}
