use crate::common;

#[test]
fn matches_cpython_coroutine_guarded_finally_borrow_cleanup() {
    for guard in ["flag", "flag is None", "flag is not None"] {
        for prefix in ["", "                replacement = error.value\n"] {
            let source = format!(
                "async def f(value, flag, other):\n    try:\n        value = await work()\n    finally:\n        prepared = await prepare(value)\n        try:\n            recover(prepared)\n        except Error as error:\n            if {guard}:\n{prefix}                if other and value is not None:\n                    consume(value)\n                else:\n                    value = error\n    return value\n"
            );
            common::assert_matches_cpython_marshal(&source, "coroutine_guarded_finally_cleanup.py");
        }
    }
}
