use crate::common;

#[test]
fn folds_literal_arms_in_negated_comprehension_filters() {
    for condition in [
        "not (left if flag else False)",
        "not (left if flag else True)",
        "not (False if flag else right)",
        "not (True if flag else right)",
        "not not (left if flag else False)",
        "item is not None and (flag or not (item.value if ready(item) else False))",
    ] {
        for (open, close) in [("[", "]"), ("(", ")")] {
            for multiline in [false, true] {
                let expression = if multiline {
                    format!(
                        "{open}\n        item\n        for item in values\n        if {condition}\n    {close}"
                    )
                } else {
                    format!("{open}item for item in values if {condition}{close}")
                };
                let source =
                    format!("def function(values, left, right, flag):\n    return {expression}\n");
                common::assert_matches_cpython_marshal(&source, "negated_literal_filter.py");
            }
        }
    }
}

#[test]
fn preserves_manager_cleanup_stack_depth_after_rejected_dictionary_filters() {
    for managers in [
        "with manager()",
        "with manager(), other()",
        "async with manager()",
        "async with manager(), other()",
    ] {
        let source = format!(
            "async def function(values, flag, value):\n    {managers}:\n        result = {{item: value for item in values if not (flag or True)}}\n        consume(result)\n"
        );
        common::assert_matches_cpython_marshal(&source, "rejected_dictionary_cleanup_depth.py");
    }
}
