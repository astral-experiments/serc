use crate::common;

#[test]
fn matches_cpython_handlers_in_copied_finally_bodies() {
    for prefix in ["", "async "] {
        for body in ["raise Error", "return None"] {
            for handler in ["Exception", "Exception as error"] {
                for returned in ["return", "return None", "return value"] {
                    let source = format!(
                        "{prefix}def f(flag, value):\n    try:\n        {body}\n    except {handler}:\n        {returned}\n    finally:\n        try:\n            if flag:\n                cleanup()\n        except Exception:\n            pass\n",
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "copied_finally_handler_reachability.py",
                    );
                }
            }
        }
    }
}
