use crate::common::assert_matches_cpython_marshal;

#[test]
fn guarded_named_handlers_reach_linear_finally_cleanup() {
    for scope in ["def", "async def"] {
        for guard in ["x", "x and y", "x or y", "x is None"] {
            for body in ["pass", "work(x)"] {
                for exit in ["raise", "raise F from error", "return x"] {
                    for cleanup in ["x.field += 1", "use(x)", "x.field = y"] {
                        for prefix in ["", "except E:\n        pass\n    "] {
                            let source = format!(
                                "{scope} f(x, y):\n    try:\n        {body}\n    {prefix}except F as error:\n        if {guard}:\n            {exit}\n    finally:\n        {cleanup}\n"
                            );
                            assert_matches_cpython_marshal(
                                &source,
                                "guarded_named_handler_finally.py",
                            );
                        }
                    }
                }
            }
        }
    }
}
