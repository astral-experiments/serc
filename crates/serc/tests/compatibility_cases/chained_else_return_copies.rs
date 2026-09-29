use crate::common;

#[test]
fn keeps_large_chained_return_blocks_after_comparison_cleanup() {
    for returned in [
        "holder.values[index]",
        "values[index].replace(' ', '_')",
        "work(index)",
    ] {
        let source = format!(
            "def f(holder, index, bound, values):\n    if 0 <= index < bound:\n        return {returned}\n    else:\n        raise IndexError('x')\n"
        );
        common::assert_matches_cpython_marshal(&source, "large_chained_else_return.py");
    }
}
