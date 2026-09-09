use crate::common;

#[test]
fn folds_debug_before_callable_and_starred_argument_lowering() {
    for header in ["def", "async def"] {
        for call in [
            "__debug__()",
            "__debug__(value)",
            "__debug__(value=value)",
            "__debug__(*values)",
            "__debug__(**options)",
            "consume(*__debug__)",
        ] {
            for protected in [false, true] {
                let body = if protected {
                    format!("try:\n    {call}\nexcept Error:\n    pass")
                } else {
                    call.to_string()
                };
                let source = format!(
                    "{header} f(value, values, options):\n    {}\n",
                    body.replace('\n', "\n    ")
                );
                common::assert_matches_cpython_marshal(&source, "debug_callable_constants.py");
            }
        }
    }
}
