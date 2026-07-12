#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_generator_iterator_negated_name_ifexp_owners() {
    let source = r#"def attrs_shape(source):
    flag = False
    for value in source:
        yield value if not flag else "_" + value
        flag = not flag

def trailing(source, flag):
    for value in source:
        yield value if not flag else prefix + value

def assigned(source, flag):
    for value in source:
        result = value if not flag else prefix + value
        yield result

def after_yield(source, flag):
    yield None
    for value in source:
        yield value if not flag else prefix + value

def in_with(source, flag, manager):
    with manager:
        for value in source:
            yield value if not flag else prefix + value

async def async_generator(source, flag):
    for value in source:
        yield value if not flag else prefix + value

async def async_for(source, flag):
    async for value in source:
        yield value if not flag else prefix + value

def call_test_control(source):
    for value in source:
        yield value if keep(value) else prefix + value

def compare_test_control(source):
    for value in source:
        yield value if value in accepted else prefix + value

def none_test_control(source):
    for value in source:
        yield value if value is not None else "missing"

def expression_ifexp_control(source, flag):
    for value in source:
        consume(value if not flag else prefix + value)
        yield value

def direct_yield_control(flag, value):
    yield value if not flag else prefix + value

def in_try_control(source, flag):
    for value in source:
        try:
            yield value if not flag else prefix + value
        except Error:
            recover(value)

def generator_expression_control(source, flag):
    return (value if not flag else prefix + value for value in source)

def synchronous_control(source, flag):
    for value in source:
        consume(value if not flag else prefix + value)
"#;
    common::assert_matches_cpython_marshal(
        source,
        "generator_iterator_negated_name_ifexp_owners.py",
    );
}
