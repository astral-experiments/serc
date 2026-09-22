use crate::common;

#[test]
fn matches_cpython_nested_finally_iterator_return_owners() {
    for (definition, suspension) in [
        ("def", ""),
        ("def", "    yield 0\n"),
        ("async def", "    await prepare()\n"),
        ("async def", "    yield 0\n"),
    ] {
        for handler in ["Error", "Error as error"] {
            let source = format!(
                "{definition} f(items):\n{suspension}    try:\n        try:\n            for item in items:\n                try:\n                    work(item)\n                except {handler}:\n                    failed()\n                    return\n        finally:\n            cleanup()\n    finally:\n        finish()\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "nested_finally_iterator_return_owners.py",
            );
        }
    }
}
