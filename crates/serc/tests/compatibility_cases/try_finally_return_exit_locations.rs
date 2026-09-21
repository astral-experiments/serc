use crate::common;

#[test]
fn matches_cpython_try_else_guarded_return_finally_exit_locations() {
    for value in ["result", "build(result)", "(result, 1)"] {
        for cleanup in ["result = None", "close(result)"] {
            for prefix in ["", "prepare(result)\n                "] {
                let source = format!(
                    "def f(flag):\n    try:\n        result = work()\n    except Exception:\n        recover()\n    else:\n        if flag:\n            try:\n                {prefix}return {value}\n            finally:\n                {cleanup}\n    finish(result)\n    return result\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "try_else_guarded_return_finally_exit.py",
                );
            }
        }
    }
}
