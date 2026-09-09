use crate::common;

#[test]
fn matches_cpython_handler_return_after_inner_try() {
    for (scope, suspension) in [("def", ""), ("async def", ""), ("def", "    yield value\n")] {
        for returned in ["value", "obj.attr"] {
            for conditional in [false, true] {
                let returned = if conditional {
                    format!("            if flag:\n                return {returned}\n")
                } else {
                    format!("            return {returned}\n")
                };
                let source = format!(
                    "{scope} f(value, flag, obj):\n{suspension}    try:\n        work()\n    except RuntimeError:\n        try:\n{returned}        except LookupError:\n            fallback()\n"
                );
                common::assert_matches_cpython_marshal(&source, "inner_try_handler_return.py");
            }
        }
    }
}
