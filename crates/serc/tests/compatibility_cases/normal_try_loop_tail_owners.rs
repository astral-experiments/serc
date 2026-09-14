use crate::common;

#[test]
fn nested_normal_try_backedges_keep_the_outer_owner() {
    for (definition, footer) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield values\n"),
    ] {
        for header in ["for value in values", "while value"] {
            for handler in ["failed(value)", "failed(value)\nraise", "return"] {
                let handler = handler.replace('\n', "\n            ");
                let source = format!(
                    "{definition} f(values, value):\n    {header}:\n        try:\n            if value:\n                if other(value):\n                    consume(value)\n                else:\n                    third(value)\n            else:\n                fourth(value)\n        except Exception:\n            {handler}\n{footer}"
                );
                common::assert_matches_cpython_marshal(&source, "normal_try_loop_tail_owners.py");
            }
        }
    }
}

#[test]
fn nested_loops_and_handlers_keep_their_own_backedge_owners() {
    for nested in [
        "for other in values:\n    if other:\n        consume(other)\n    else:\n        third(other)",
        "try:\n    if other(value):\n        consume(value)\n    else:\n        third(value)\nexcept OtherError:\n    failed(value)",
    ] {
        let nested = nested.replace('\n', "\n                ");
        let source = format!(
            "def f(values):\n    for value in values:\n        try:\n            if value:\n                {nested}\n            else:\n                fourth(value)\n        except Exception:\n            failed(value)\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_loop_tail_owner_boundaries.py");
    }
}

#[test]
fn nested_named_try_exits_keep_the_coroutine_loop_owner() {
    let source = r"async def worker():
    while True:
        try:
            item = claim()
            if item is None:
                await pause()
                continue
            context = build(item)
            try:
                await execute(context)
            except Exception as error:
                report(error)
                update(item)
        except CancelledError:
            raise
        except Exception:
            report_failure()
";
    common::assert_matches_cpython_marshal(source, "nested_try_while_exit_owners.py");
}
