use crate::common;

#[test]
fn matches_cpython_with_exit_after_handler_finally() {
    for (prefix, suspension) in [
        ("", ""),
        ("", "    yield value\n"),
        ("async ", "    await before()\n"),
    ] {
        for body in ["work(value)", "with inner():\n    work(value)"] {
            for handler in ["report()", "raise", "report()\nreturn", "pass"] {
                for cleanup in ["cleanup()", "value = None", "cleanup()\nreport()"] {
                    let body = body.replace('\n', "\n            ");
                    let handler = handler.replace('\n', "\n            ");
                    let cleanup = cleanup.replace('\n', "\n            ");
                    let source = format!(
                        "{prefix}def example(manager, value):\n{suspension}    with manager:\n        try:\n            {body}\n        except Error:\n            {handler}\n        finally:\n            {cleanup}\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "with_handler_finally_exit_nops.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_with_finally_branch_join_ownership() {
    for (prefix, suspension, body) in [
        ("", "", "work(value)"),
        ("", "", "for item in values:\n    work(item)"),
        (
            "",
            "    yield value\n",
            "for item in values:\n    work(item)",
        ),
        (
            "async ",
            "    await before()\n",
            "for item in values:\n    work(item)",
        ),
    ] {
        for after in ["", "    after()\n"] {
            let body = body.replace('\n', "\n            ");
            let source = format!(
                "{prefix}def example(manager, value, values, flag):\n{suspension}    with manager:\n        try:\n            {body}\n        except Error:\n            raise\n        finally:\n            if flag:\n                cleanup()\n            else:\n                report()\n{after}"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "with_finally_branch_join_ownership.py",
            );
        }
    }
}
