use crate::common;

#[test]
fn matches_cpython_handler_assignment_join_before_loop_condition() {
    for (function, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield flag\n"),
        ("async def", "    yield flag\n"),
    ] {
        for loop_header in ["for item in items:", "while active:"] {
            for name in ["", " as error"] {
                for condition in ["flag", "True"] {
                    for body in ["value = other", "value.attr = other", "value.attr += other"] {
                        let source = format!(
                            "{function} f(items, active, flag, done, other, value):\n{suspension}    {loop_header}\n        try:\n            work()\n        except Exception{name}:\n            if {condition}:\n                {body}\n        if done:\n            break\n    return\n"
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "handler_loop_assignment_join_locations.py",
                        );
                    }
                }
            }
        }
    }
}
