use crate::common;

#[test]
fn folds_repeated_negation_at_multiline_value_boundaries() {
    let source = r"def standalone(flag, other, value):
    return (
        not not value
    )

def and_value(flag, other, value):
    return (
        not flag and (not not value)
    )

def or_assignment(flag, other, value):
    result = (
        not flag or (not not value)
    )
    return result

def call_value(flag, other, value):
    return work((
        not not value
    ))

def possibly_unbound(flag, other):
    if flag:
        value = other
    return (
        not not value
    )

def same_line_control(flag, other, value):
    return not not value
";
    common::assert_matches_cpython_marshal(source, "multiline_value_negation.py");
}

#[test]
fn preserves_distinct_lines_when_folding_repeated_negation() {
    let source = r"def split_double(flag, other, value):
    return (
        not flag and not
        not
        value
    )

def split_triple(flag, other, value):
    return (
        not flag and not
        not
        not
        value
    )

def split_quadruple(flag, other, value):
    return (
        not flag and not
        not
        not
        not
        value
    )

def return_line_split(flag, other, value):
    return (not flag and not
        not not value
    )
";
    common::assert_matches_cpython_marshal(source, "split_unary_negation.py");
}

#[test]
fn folds_repeated_negation_in_protected_async_and_generator_bodies() {
    let source = r"def try_return(flag, value, values):
    try:
        return (
            not
            not
            value
        )
    except Error:
        recover()

def handler_assignment(flag, value, values):
    try:
        work()
    except Error as error:
        result = (
            not not value
        )
        consume(result)
        recover(error)

def finally_assignment(flag, value, values):
    try:
        work()
    finally:
        result = (
            not not value
        )
        consume(result)

async def async_manager(flag, value, values):
    async with manager():
        result = (
            not flag and (not not value)
        )
        consume(result)

def generator_loop(flag, value, values):
    if flag:
        yield value
    for item in values:
        return (
            not
            not
            value
        )

async def async_generator_loop(flag, value, values):
    if flag:
        yield value
    async for item in values:
        result = (
            not not value
        )
        consume(result)
";
    common::assert_matches_cpython_marshal(source, "protected_value_negation.py");
}
