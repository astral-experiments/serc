use crate::common;

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
