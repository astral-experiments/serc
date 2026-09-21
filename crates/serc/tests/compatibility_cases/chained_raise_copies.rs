use crate::common;

#[test]
fn preserves_protected_conditional_raise_owners() {
    let source = r"def function(first, second):
    try:
        if first:
            if second:
                error = ValueError if first else TypeError
            else:
                error = TypeError if first else ValueError
            raise error from None
        return None
    except Exception:
        recover()
";
    common::assert_matches_cpython_marshal(source, "protected_conditional_raise.py");
}

#[test]
fn respects_original_slot_limit_for_copied_raises() {
    for raised in [
        "raise error",
        "raise error from None",
        "raise error from cause",
        "raise ValueError(error)",
    ] {
        let source = format!(
            "def function(first, second, cause):
    if first:
        if second:
            error = ValueError if first else TypeError
        else:
            error = TypeError if first else ValueError
        {raised}
    return None
"
        );
        common::assert_matches_cpython_marshal(&source, "copied_raise_original_size.py");
    }
}
