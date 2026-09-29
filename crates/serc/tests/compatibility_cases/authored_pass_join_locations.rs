use crate::common;

#[test]
fn matches_cpython_noop_loop_finally_after_raising_handlers() {
    for handler in ["raise", "raise TypeError()"] {
        for exception in ["Exception", "Exception as error"] {
            let source = format!(
                "def f(x, y):\n    for q in y:\n        try:\n            x()\n        except {exception}:\n            {handler}\n        finally:\n            pass\n"
            );
            common::assert_matches_cpython_marshal(&source, "raising_handler_pass_finally.py");
        }
    }
}
