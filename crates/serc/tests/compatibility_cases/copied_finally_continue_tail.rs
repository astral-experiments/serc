use crate::common;

#[test]
fn normalizes_conditional_finally_copies_before_continue() {
    for (definition, prefix) in [("def", ""), ("async def", ""), ("def", "    yield None\n")] {
        for loop_header in ["for item in items", "while flag"] {
            for selector in ["other", "not other"] {
                for protected_body in [
                    "        try:\n            continue\n",
                    "        try:\n            use()\n        except ValueError:\n            continue\n",
                ] {
                    let source = format!(
                        "{definition} f(items, flag, other):\n{prefix}    {loop_header}:\n{protected_body}        finally:\n            if {selector}:\n                cleanup()\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "copied_finally_continue_tail.py",
                    );
                }
            }
        }
    }
}

#[test]
fn normalizes_continue_tail_when_try_encloses_the_whole_loop() {
    for (definition, prefix) in [("def", ""), ("async def", ""), ("def", "    yield None\n")] {
        for loop_header in ["for item in items", "while other"] {
            let source = format!(
                "{definition} f(items, flag, other):\n{prefix}    try:\n        {loop_header}:\n            try:\n                work()\n            except LookupError:\n                continue\n            finally:\n                if flag:\n                    action()\n    except BaseException:\n        recover()\n"
            );
            common::assert_matches_cpython_marshal(&source, "copied_finally_enclosing_loop.py");
        }
    }
}
