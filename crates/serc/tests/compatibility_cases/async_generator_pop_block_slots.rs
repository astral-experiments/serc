use crate::common;

#[test]
fn matches_cpython_async_generator_pop_block_slots() {
    for (callable, target, condition_prefix) in [
        ("all", "elem", ""),
        ("any", "elem", ""),
        ("tuple", "elem", ""),
        ("all", "(elem, other)", ""),
        ("all", "(elem, *other)", ""),
        ("all", "elem", "not "),
        ("all", "elem", "elem if "),
    ] {
        let source = format!(
            "def f(iterable, converted, {callable}):
    return (elem async for {target} in iterable
            if {condition_prefix}{callable}(pred(value) for pred, value in converted))
"
        );
        common::assert_matches_cpython_marshal(&source, "async_generator_pop_slots.py");
    }
}
