use crate::common;

#[test]
fn matches_cpython_conditional_comprehension_filters() {
    for condition in [
        "(item if flag else other)",
        "(check(item) if flag else other is None)",
        "(item is None if flag else item > 0)",
        "not (item if flag else other)",
        "((item or other) if flag else (item and other))",
        "(item if flag else True)",
        "(item if True else other)",
        "(item if False else other)",
    ] {
        for collection in [
            "[item for item in items if CONDITION]",
            "{item for item in items if CONDITION}",
            "{item: item for item in items if CONDITION}",
            "(item for item in items if CONDITION)",
        ] {
            let expression = collection.replace("CONDITION", condition);
            for protected in [false, true] {
                let source = if protected {
                    format!(
                        "def f(items, flag, other):\n    try:\n        result = {expression}\n        consume(result)\n    except Error:\n        pass\n"
                    )
                } else {
                    format!(
                        "def f(items, flag, other):\n    result = {expression}\n    consume(result)\n"
                    )
                };
                common::assert_matches_cpython_marshal(&source, "conditional_filters.py");
            }
        }
    }
}
