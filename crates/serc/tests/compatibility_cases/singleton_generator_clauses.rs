use super::super::common::assert_matches_cpython_marshal;

#[test]
fn singleton_generator_clauses_match_cpython_marshal() {
    let source = r"def list_tuple_target(values):
    return (value for item in values for key, _, value in [item.partition('=')] if key == 'selected')

def tuple_singleton(values, transform):
    return (value for item in values for value in (transform(item),))

def multiple_filters(values, transform, accept):
    return (value for item in values if item for value in [transform(item)] if accept(value) if value is not None)

def following_loop(values, transform, expand):
    return (result for item in values for value in [transform(item)] for result in expand(value))

def successive_singletons(values, first, second):
    return (result for item in values for value in [first(item)] if value for result in (second(value),) if result)

async def async_outer(values, transform):
    return (value async for item in values for value in [transform(item)] if value)

async def async_inner(values, transform, expand):
    return (result for item in values for value in [transform(item)] async for result in expand(value))

async def awaited_singleton(values, transform):
    return (value for item in values for value in [await transform(item)] if value)

def starred_control(values, expand):
    return (value for item in values for value in [*expand(item)])

def multiple_control(values, first, second):
    return (value for item in values for value in [first(item), second(item)])

def outer_control(value):
    return (item for item in [value])
";
    assert_matches_cpython_marshal(source, "singleton_generator_clauses.py");
}
