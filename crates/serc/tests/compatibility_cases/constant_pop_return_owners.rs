use crate::common;

#[test]
fn matches_cpython_constant_pop_return_owners() {
    for (definition, suspension) in [
        ("def", ""),
        ("def", "    yield 0\n"),
        ("async def", "    await work()\n"),
        ("async def", "    yield 0\n"),
    ] {
        for builtin in ["any", "all"] {
            let source = format!(
                "{definition} f(values):\n{suspension}    try:\n        {builtin}(x for x in values)\n    except Error:\n        raise\n"
            );
            common::assert_matches_cpython_marshal(&source, "constant_pop_return_owners.py");
        }
    }
}
