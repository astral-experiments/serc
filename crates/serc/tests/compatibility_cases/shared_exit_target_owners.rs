use crate::common;

#[test]
fn matches_cpython_shared_named_handler_exit_owners() {
    for (definition, suspension) in [
        ("def", ""),
        ("def", "    yield 0\n"),
        ("async def", "    await prepare()\n"),
        ("async def", "    yield 0\n"),
    ] {
        for marker in ["", "                    pass\n"] {
            let source = format!(
                "{definition} f(values, flag):\n{suspension}    try:\n        work()\n    except Error as error:\n        touch(error)\n        if flag:\n            for value in values:\n                if value:\n{marker}                    break\n    else:\n        after()\n"
            );
            common::assert_matches_cpython_marshal(&source, "shared_named_handler_exit_owners.py");
        }
    }
}

#[test]
fn matches_cpython_shared_exit_preserves_source_nop_owner() {
    for marker in ["", "                    pass\n"] {
        let source = format!(
            "def f(value, flag):\n    try:\n        work()\n    except Error as error:\n        touch(error)\n        if flag:\n            while value:\n                if value:\n{marker}                    break\n    else:\n        after()\n"
        );
        common::assert_matches_cpython_marshal(&source, "shared_exit_source_nop_owner.py");
    }
}
