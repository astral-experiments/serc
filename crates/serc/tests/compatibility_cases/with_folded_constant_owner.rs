use crate::common;

#[test]
fn cfg_folded_with_body_constants_keep_the_authored_exception_owner() {
    for expression in [
        "1 + 1",
        "-3",
        "(1, 2)",
        "f'x'",
        "not True",
        "(1, 2)[0]",
        "2",
        "pass",
    ] {
        for (prefix, continuation) in [
            ("def f(manager):", ""),
            ("async def f(manager):", "    await wait()\n"),
        ] {
            let source =
                format!("{prefix}\n    with manager:\n        {expression}\n{continuation}");
            common::assert_matches_cpython_marshal(&source, "with_folded_constant_owner.py");
        }
        let source = format!(
            "def f(manager):\n    try:\n        with manager:\n            {expression}\n    except Exception:\n        pass\n"
        );
        common::assert_matches_cpython_marshal(&source, "with_folded_constant_outer_owner.py");
    }
}
