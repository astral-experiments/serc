use crate::common;

#[test]
fn matches_cpython_direct_handler_break_return_owners() {
    for (definition, suspension) in [
        ("def", ""),
        ("def", "    yield 0\n"),
        ("async def", "    await prepare()\n"),
        ("async def", "    yield 0\n"),
    ] {
        for following in ["", "    finish()\n"] {
            let source = format!(
                "{definition} f():\n{suspension}    while True:\n        try:\n            try:\n                work()\n            except Inner:\n                break\n            after()\n        except Outer:\n            recover()\n{following}"
            );
            common::assert_matches_cpython_marshal(&source, "direct_handler_break_owners.py");
        }
    }
}
