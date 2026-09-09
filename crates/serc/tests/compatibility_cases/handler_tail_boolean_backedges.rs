use crate::common;

#[test]
fn matches_cpython_handler_terminating_branch_boolean_backedges() {
    for definition in ["def", "async def"] {
        for condition in ["flag and value is not None", "flag and ready(value)"] {
            for exit in ["return value", "continue", "break", "raise ValueError"] {
                for handler in ["except Exception:", "except Exception as error:"] {
                    let source = format!(
                        "{definition} f(items, flag):\n    for value in items:\n        try:\n            observe(value)\n        {handler}\n            if {condition}:\n                {exit}\n    return None\n"
                    );
                    common::assert_matches_cpython_marshal(&source, "handler_tail_boolean.py");
                }
            }
        }
    }
}
