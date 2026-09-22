use crate::common;

#[test]
fn matches_cpython_single_finally_pop_before_copied_break_condition() {
    for (definition, prefix, loop_header, tail) in [
        ("async def", "", "while flag", ""),
        ("async def", "", "for item in values", ""),
        ("async def", "", "async for item in values", ""),
        (
            "async def",
            "    yield flag\n",
            "async for item in values",
            "",
        ),
        ("async def", "", "while flag", "    yield flag\n"),
        ("def", "", "while flag", "    yield flag\n"),
        ("def", "", "while flag", ""),
    ] {
        for condition in ["flag", "flag is None", "flag and values", "flag or values"] {
            for cleanup in ["pass", "consume(flag)"] {
                let source = format!(
                    "{definition} f(values, flag):\n{prefix}    {loop_header}:\n        try:\n            if {condition}:\n                break\n        finally:\n            if flag:\n                {cleanup}\n{tail}"
                );
                common::assert_matches_cpython_marshal(&source, "copied_finally_break_owner.py");
            }
        }
    }
}
