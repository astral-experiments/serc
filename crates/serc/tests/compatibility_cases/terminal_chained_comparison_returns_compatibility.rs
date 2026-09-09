use crate::common;

#[test]
fn matches_cpython_marshal_for_terminal_chained_comparison_returns() {
    let source = r"def name_return(low, value, high):
    if low <= value < high:
        return value

def attribute_bound(start, value, end):
    if start <= value < end.limit:
        return value

def binary_bound(start, value, end, step):
    if start <= value < end - step:
        return value

def literal_return(start, value, end):
    if start <= value < end:
        return True

def none_return(start, value, end):
    if start <= value < end:
        return None

def long_chain(start, value, middle, end):
    if start <= value < middle <= end:
        return value

def preceding_condition(start, value, middle, end):
    if start <= value < middle:
        return start
    if start <= value < end:
        return value

def preceding_boolean(start, value, middle, end):
    if value < start or value >= end:
        return start
    if start <= value < middle:
        return value

def preceding_work(start, value, end):
    record(value)
    if start <= value < end:
        return value

def final_none_comparator(left, right):
    if left is None != right is None:
        return left

def final_not_none_comparator(left, right):
    if left is not None == right is not None:
        return right

def deep_attribute_control(start, value, end):
    if start <= value < end:
        return value.one.two.three.four

def complex_subscript_control(start, value, end):
    if start <= value < end:
        return value[convert(end)]

def positional_call_control(start, value, end):
    if start <= value < end:
        return factory(value)

def keyword_call_control(start, value, end):
    if start <= value < end:
        return factory(value=value)

def nested_call_control(start, value, end):
    if start <= value < end:
        return factory(convert(value))

def method_call_control(start, value, end):
    if start <= value < end:
        return value.with_value(start)

def binary_attribute_control(start, value, end):
    if start <= value < end:
        return value.left + end.right

def binary_call_control(start, value, end):
    if start <= value < end:
        return convert(value) + convert(end)

def tuple_call_control(start, value, end):
    if start <= value < end:
        return convert(value), convert(end)

def ifexp_control(start, value, end, flag):
    if start <= value < end:
        return value if flag else end

def boolop_control(start, value, end):
    if start <= value < end:
        return value and end

def raise_control(start, value, end):
    if start <= value < end:
        raise ValueError(value)

def call_body_control(start, value, end):
    if start <= value < end:
        record(value)

def nonterminal_control(start, value, end):
    if start <= value < end:
        return value
    return end

def loop_control(values, start, end):
    for value in values:
        if start <= value < end:
            return value
    return end

async def coroutine(start, value, end):
    await prepare(value)
    if start <= value < end:
        return value

async def await_control(start, value, end):
    if start <= value < end:
        return await value

def generator(start, value, end):
    yield start
    if start <= value < end:
        return value

def protected(manager, start, value, end):
    with manager:
        record(value)
    if start <= value < end:
        return value
";
    common::assert_matches_cpython_marshal(source, "terminal_chained_comparison_returns.py");
}

#[test]
fn matches_cpython_marshal_for_terminal_chained_comparison_assignment_locations() {
    let source = r"def attribute_chain(x, obj):
    if 0 < x < 10:
        obj.value = x

def name_chain(x):
    if 0 < x < 10:
        result = x

def subscript_chain(x, items):
    if 0 < x < 10:
        items[0] = x

def augmented_attribute_chain(x, obj):
    if 0 < x < 10:
        obj.value += x

def augmented_name_chain(x, obj):
    if 0 < x < 10:
        x += 1

def augmented_subscript_chain(x, obj):
    if 0 < x < 10:
        obj[0] += x

def augmented_slice_chain(x, obj):
    if 0 < x < 10:
        obj[0:2] += obj

async def async_attribute_chain(x, obj):
    if 0 < x < 10:
        obj.value = x

def attribute_single(x, obj):
    if x < 10:
        obj.value = x

def attribute_chain_nonterminal(x, obj):
    if 0 < x < 10:
        obj.value = x
    result = x

def explicit_return_chain(x, obj):
    if 0 < x < 10:
        return x
";
    common::assert_matches_cpython_marshal(
        source,
        "terminal_chained_comparison_assignment_locations.py",
    );
}
