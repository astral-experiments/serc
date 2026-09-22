use crate::common;

#[test]
fn copied_comprehension_exit_stores_apply_original_static_swaps() {
    for (comprehension, fallback) in [
        ("{key: value for key, value in pairs}", "{}"),
        ("[key for key, value in pairs]", "[]"),
        ("{key for key, value in pairs}", "set()"),
        ("{key: key for key in pairs}", "{}"),
    ] {
        for (body, alternate) in [(comprehension, fallback), (fallback, comprehension)] {
            for tail in [
                "return result",
                "return other",
                "consume(result)\n    return result",
            ] {
                let source = format!(
                    "def f(pairs, flag, other):\n    result = {body} if flag else {alternate}\n    {tail}\n"
                );
                common::assert_matches_cpython_marshal(&source, "copied_comprehension_store.py");
            }
        }
    }
}
