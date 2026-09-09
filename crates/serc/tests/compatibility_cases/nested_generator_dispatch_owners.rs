use crate::common;

#[test]
fn matches_cpython_marshal_for_nested_generator_dispatch_owners() {
    let source = r"def ordinary_consumer(values):
    return sum(any(item > value for item in values) for value in values)


def tuple_consumer(values):
    return tuple(any(item > value for item in values) for value in values)


def unpacked_target(values):
    return all(any(item > left + right for item in values) for left, right in values)


def tuple_element(values):
    return list(tuple(item for item in values) for value in values)


def prior_branch(values, ready):
    return list(ready or any(item > value for item in values) for value in values)


def call_argument_keeps_slot(values):
    return any(consume(any(item > value for item in values)) for value in values)


def plain_target_control(values):
    return any(any(item > value for item in values) for value in values)
";
    common::assert_matches_cpython_marshal(source, "nested_generator_dispatch_owners.py");
}
