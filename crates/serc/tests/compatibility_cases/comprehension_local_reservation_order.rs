use crate::common;

#[test]
fn matches_cpython_marshal_for_comprehension_local_reservation_order() {
    let source = r"def global_then_temporary(items):
    consume(item)
    page = load(items)
    return [item for item in page]

def walrus_in_element(groups):
    return [((identifier := group.id), [item.value for item in group.items]) for group in groups]

def walrus_in_filter(groups):
    return [[item.value for item in group.items] for group in groups if (identifier := group.id)]

def nested_filter_before_later_target(items):
    return [(item, other) for item in items if [nested for nested in item] for other in item]

def global_then_captured_temporary(items):
    consume(nested)
    return [lambda: nested for item in items for nested in item]

def captured_temporary_with_later_binding(items):
    consume(item)
    page = items
    result = [lambda: item for item in page]
    item = None
    return result

def filter_reads_later_target(items):
    return [(first, second, third) for first in items if third for second in first for third in second]

def nested_first_iterable(items):
    return [[[value for value in group] for group in [entry for entry in item]] for item in items]

def dict_value_before_key(items):
    return {tuple([key for key in item]): [value for value in item] for item in items}
";
    common::assert_matches_cpython_marshal(source, "comprehension_local_reservation_order.py");
}
