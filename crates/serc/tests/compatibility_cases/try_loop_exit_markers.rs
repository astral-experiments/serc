use crate::common;

#[test]
fn matches_cpython_try_loop_exit_markers() {
    for header in [
        "for item in values:",
        "async for item in values:",
        "while values:",
        "while not values:",
    ] {
        for body in ["consume(item)", "if predicate(item):\n    return item"] {
            for handler in ["pass", "return None", "raise Error"] {
                let prefix = if header.starts_with("async") {
                    "async "
                } else {
                    ""
                };
                let source = format!(
                    "{prefix}def f(values):\n    try:\n        values = prepare(values)\n        {header}\n            {}\n    except Error:\n        {handler}\n    return values\n",
                    body.replace('\n', "\n            "),
                );
                common::assert_matches_cpython_marshal(&source, "try_loop_exit_markers.py");
            }
        }
    }
}
