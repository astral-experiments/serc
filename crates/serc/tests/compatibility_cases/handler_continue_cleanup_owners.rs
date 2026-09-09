use crate::common;

#[test]
fn handler_continues_end_enclosing_tries_after_exception_cleanup() {
    for prefix in ["", "async "] {
        for name in ["", " as error"] {
            for loop_header in ["while flag:", "for item in items:"] {
                for nested in [false, true] {
                    let handler = format!(
                        "try:\n    work()\nexcept Error{name}:\n    observe()\n    continue"
                    );
                    let mut body = format!(
                        "try:\n    {}\nexcept Other:\n    recover()",
                        handler.replace('\n', "\n    "),
                    );
                    if nested {
                        body = format!(
                            "try:\n    {}\nexcept Outer:\n    recover()",
                            body.replace('\n', "\n    "),
                        );
                    }
                    let source = format!(
                        "{prefix}def f(flag, items):\n    {loop_header}\n        {}\n    finish()\n",
                        body.replace('\n', "\n        "),
                    );
                    common::assert_matches_cpython_marshal(&source, "handler_continue_owner.py");
                }
            }
        }
    }
}

#[test]
fn handler_continue_after_await_keeps_only_the_coroutine_jump_owner() {
    for name in ["", " as error"] {
        let source = format!(
            "async def f():\n    while True:\n        try:\n            await sleep()\n            try:\n                work()\n            except Error{name}:\n                observe()\n                continue\n        except Other:\n            break\n"
        );
        common::assert_matches_cpython_marshal(&source, "await_handler_continue_owner.py");
    }
}
