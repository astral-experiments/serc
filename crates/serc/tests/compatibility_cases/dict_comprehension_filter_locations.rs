use crate::common;

#[test]
fn matches_cpython_dict_comprehension_filter_locations() {
    for (prefix, outer, inner) in [
        ("", "for item in items", "for key in other"),
        ("", "for item in items", "for key in (item,)"),
        ("", "for item in items", "for key in [item]"),
        ("async ", "async for item in items", "for key in other"),
    ] {
        for condition in ["flag", "value is not None", "not flag"] {
            for filter_outer in [false, true] {
                let outer_filter = if filter_outer { " if " } else { "" };
                let outer_condition = if filter_outer { condition } else { "" };
                let inner_filter = if filter_outer { "" } else { " if " };
                let inner_condition = if filter_outer { "" } else { condition };
                let source = format!(
                    "{prefix}def f(items, other, value, flag):\n    return {{\n        key: value\n        {outer}{outer_filter}{outer_condition}\n        {inner}{inner_filter}{inner_condition}\n    }}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "dict_comprehension_filter_locations.py",
                );
            }
        }
    }
}
