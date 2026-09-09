use crate::common;

#[test]
fn matches_cpython_exception_owners_after_async_iterator_exhaustion() {
    for prefix in [
        "",
        "    values.a = values\n",
        "    discard(values)\n",
        "    values.discard(values)\n",
    ] {
        for condition in ["flag", "not flag", "flag and values", "flag or values"] {
            let source = format!(
                "async def f(values, flag):\n    async for value in values:\n        use(value)\n{prefix}    if {condition}:\n        await go(flag)\n"
            );
            common::assert_matches_cpython_marshal(&source, "async_for_exhaustion_owners.py");
        }
    }
}

#[test]
fn matches_cpython_locations_on_copied_iterator_break_exits() {
    for iteration in [
        "async for item in source()",
        "async for item in items",
        "for item in items",
    ] {
        for body in [
            "break",
            "if flag:\n    break",
            "if flag:\n    break\nif other:\n    break",
            "if flag:\n    work(item)\nelse:\n    break",
            "yield item\nif flag:\n    break",
        ] {
            for loop_else in [
                "",
                "        else:\n            work()\n",
                "        else:\n            return\n",
            ] {
                let source = format!(
                    "async def f(items, flag, other):\n    try:\n        {iteration}:\n            {}\n{loop_else}    except Error as error:\n        recover(error)\n",
                    body.replace('\n', "\n            "),
                );
                common::assert_matches_cpython_marshal(&source, "iterator_break_exit_locations.py");
            }
        }
    }
}

#[test]
fn preserves_async_iterator_break_locations_through_finally() {
    for iteration in ["source()", "items"] {
        for body in [
            "break",
            "if flag:\n    break",
            "yield item\nif flag:\n    break",
        ] {
            for loop_else in ["", "        else:\n            return\n"] {
                let source = format!(
                    "async def f(items, flag):\n    try:\n        async for item in {iteration}:\n            {}\n{loop_else}    except Error as error:\n        recover(error)\n    finally:\n        finish()\n",
                    body.replace('\n', "\n            "),
                );
                common::assert_matches_cpython_marshal(&source, "async_break_finally_locations.py");
            }
        }
    }
}
