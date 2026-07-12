#[allow(dead_code)]
mod common;

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

#[test]
fn matches_cpython_marshal_for_nonfinal_and_no_try_else_controls() {
    let source = r"def nonfinal_control(items, mapping):
    for item in items:
        try:
            if item in mapping:
                continue
            prepare(item)
        except Error:
            recover(item)
        else:
            process(item)

def no_try_else_control(items, mapping):
    for item in items:
        try:
            if item in mapping:
                continue
        except Error:
            recover(item)
";
    common::assert_matches_cpython_marshal(
        source,
        "try_body_single_comparison_continue_controls.py",
    );
}
