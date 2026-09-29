use crate::common;

#[test]
fn matches_cpython_marshal_for_each_single_comparison_operator() {
    for (name, comparison) in [
        ("less", "item < sentinel"),
        ("less_equal", "item <= sentinel"),
        ("equal", "item == sentinel"),
        ("not_equal", "item != sentinel"),
        ("greater", "item > sentinel"),
        ("greater_equal", "item >= sentinel"),
        ("is", "item is sentinel"),
        ("is_not", "item is not sentinel"),
        ("in", "item in sentinel"),
        ("not_in", "item not in sentinel"),
    ] {
        let source = format!(
            "def compare(items, sentinel):\n    for item in items:\n        try:\n            if {comparison}:\n                continue\n        except Error:\n            recover(item)\n        else:\n            process(item)\n"
        );
        common::assert_matches_cpython_marshal(&source, &format!("{name}_continue.py"));
    }
}
