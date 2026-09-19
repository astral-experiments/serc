use crate::common;

#[test]
fn discarded_literal_slices_keep_their_codegen_constant_order() {
    for slice in [
        ":50",
        ":",
        "1:50",
        "None:50",
        "True:False",
        "1:50:2",
        "::False",
        "\"a\":\"z\"",
    ] {
        for context in ["if_false", "if_true_else", "after_return", "while_false"] {
            let discarded = format!("        consume(values[{slice}])\n        consume('later')\n");
            let body = match context {
                "if_false" => format!("    if False:\n{discarded}"),
                "if_true_else" => format!("    if True:\n        before()\n    else:\n{discarded}"),
                "after_return" => format!("    if flag:\n        return values\n{discarded}"),
                "while_false" => format!("    while False:\n{discarded}"),
                _ => unreachable!(),
            };
            let source = format!(
                "def f(values, flag):\n    consume('before')\n{body}    consume(values[{slice}], 'later')\n"
            );
            common::assert_matches_cpython_marshal(&source, "discarded_literal_slice_admission.py");
        }
    }
}
