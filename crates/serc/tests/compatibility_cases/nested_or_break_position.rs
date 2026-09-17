use crate::common;

#[test]
fn matches_cpython_marshal_for_nested_or_break_position_target() {
    let source = r"def collect(source, timeout):
    items = []
    value = None
    while True:
        item = read_next(source, timeout)
        if not item:
            break
        items.append(item)
        if len(item) >= 3 and item[:3].isdigit():
            value = int(item[:3])
            if len(item) < 4 or item[3] != '-':
                break
        else:
            break
        if len(items) >= 100:
            break
    return value, items
";
    common::assert_matches_cpython_marshal(source, "nested_or_break_position_target.py");
}

#[test]
fn matches_cpython_marshal_for_nested_or_break_position_single_test_control() {
    let source = r"def collect(source, timeout):
    items = []
    value = None
    while True:
        item = read_next(source, timeout)
        if not item:
            break
        items.append(item)
        if len(item) >= 3 and item[:3].isdigit():
            value = int(item[:3])
            if item[3] != '-':
                break
        else:
            break
        if len(items) >= 100:
            break
    return value, items
";
    common::assert_matches_cpython_marshal(
        source,
        "nested_or_break_position_single_test_control.py",
    );
}
