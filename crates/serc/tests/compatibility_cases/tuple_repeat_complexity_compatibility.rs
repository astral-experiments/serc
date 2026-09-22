use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_tuple_repetition_recursive_complexity_limit() {
    // CPython allows at most 1024 recursively contained tuple items. The outer
    // tuple's slots count too, so 4 * (1 + 255) fits but 4 * (1 + 256) does not.
    for expression in [
        "((1,) * 255,) * 4",
        "((1,) * 256,) * 3",
        "((1,) * 256,) * 4",
        "((1,) * 256,) * 5",
        "4 * ((1,) * 256,)",
        "(((1,) * 255,) * 4,) * 1",
        "(((1,) * 255,) * 4,) * 0",
        "(((1,),) * 15, 0) * 32",
        "(((1,),) * 15, 0) * 33",
    ] {
        let source = format!("value = {expression}\n");
        assert_matches_cpython_marshal(&source, "tuple_repeat_complexity.py");
        let source = format!("def f():\n    return {expression}\n");
        assert_matches_cpython_marshal(&source, "tuple_repeat_complexity_function.py");
    }
}
