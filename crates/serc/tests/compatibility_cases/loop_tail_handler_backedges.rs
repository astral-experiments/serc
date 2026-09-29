use crate::common;

#[test]
fn matches_cpython_loop_tail_handler_backedges() {
    for prefix in ["", "async "] {
        for binding in ["", " as error"] {
            for returns in [false, true] {
                for loop_header in ["for item in items:", "while ready:"] {
                    for handler_tail in [
                        "if ready:\n    recover()",
                        "if ready:\n    recover()\nelse:\n    fallback()",
                        "if ready:\n    return recover()",
                    ] {
                        let returned = if returns { "return " } else { "" };
                        let awaited = if prefix.is_empty() { "" } else { "await " };
                        let source = format!(
                            "{prefix}def f(items, ready):\n    {loop_header}\n        try:\n            {returned}{awaited}work(item)\n        except Error{binding}:\n            record()\n            {}\n    finish()\n",
                            handler_tail.replace('\n', "\n            "),
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "loop_tail_handler_backedges.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_awaited_loop_tail_handler_backedges() {
    for binding in ["", " as error"] {
        for returns in [false, true] {
            for loop_header in ["for item in items:", "while ready:"] {
                let returned = if returns { "return " } else { "" };
                let source = format!(
                    "async def f(items, ready):\n    {loop_header}\n        try:\n            {returned}await work(item)\n        except Error{binding}:\n            record()\n            if ready:\n                await recover()\n    finish()\n",
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "awaited_loop_tail_handler_backedges.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_loop_tail_handler_backedges_with_else() {
    for prefix in ["", "async "] {
        for loop_header in ["for item in items:", "while ready:"] {
            for binding in ["", " as error"] {
                for handler_tail in [
                    "if ready:\n    recover(item)",
                    "if ready:\n    recover(item)\nelse:\n    fallback(item)",
                ] {
                    for normal_tail in [
                        "completed(item)",
                        "return item",
                        "if ready:\n    completed(item)",
                    ] {
                        let awaited = if prefix.is_empty() { "" } else { "await " };
                        let source = format!(
                            "{prefix}def f(items, ready):\n    {loop_header}\n        try:\n            result = {awaited}work(item)\n        except Error{binding}:\n            record()\n            {}\n        else:\n            {}\n    finish()\n",
                            handler_tail.replace('\n', "\n            "),
                            normal_tail.replace('\n', "\n            "),
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "loop_tail_handler_backedges_with_else.py",
                        );
                    }
                }
            }
        }
    }
}
