use crate::common;

#[test]
fn matches_cpython_marshal_for_terminal_chained_none_conditions() {
    let source = r#"def terminal_is_none_raise(left, right):
    if left is None != right is None:
        raise ValueError("invalid")

def terminal_is_not_none_raise(left, right):
    if left is not None == right is not None:
        raise ValueError("invalid")

def terminal_ordered_is_none(left, right):
    if left < right is None:
        raise ValueError("invalid")

def terminal_ordered_is_not_none(left, right):
    if left < right is not None:
        raise ValueError("invalid")

def terminal_membership_is_none(left, right):
    if left in right is None:
        raise ValueError("invalid")

def terminal_long_chain(left, middle, right):
    if left is None != middle < right is None:
        raise ValueError("invalid")

def terminal_call_condition(factory, value):
    if factory() == value is None:
        raise ValueError("invalid")

def terminal_attribute_condition(left, value):
    if left.value == value is not None:
        raise ValueError("invalid")

def terminal_nonterminating_body(left, right):
    if left < right is None:
        record(left, right)

def terminal_multiple_body_statements(left, right):
    if left < right is not None:
        record(left)
        raise ValueError(right)

def nonterminal_chain_control(left, right):
    if left is None != right is None:
        record(left)
    record(right)

def ordinary_chain_control(left, middle, right):
    if left < middle <= right:
        raise ValueError("invalid")

def ordinary_none_control(value):
    if value is None:
        raise ValueError("invalid")

async def coroutine_terminal(left, right):
    await prepare(left)
    if left is None != right is None:
        raise ValueError("invalid")

def generator_terminal(left, right):
    yield left
    if left is not None == right is not None:
        raise ValueError("invalid")

def protected_terminal(manager, left, right):
    with manager:
        record(left)
    if left is None != right is None:
        raise ValueError("invalid")
"#;
    common::assert_matches_cpython_marshal(source, "terminal_chained_none_conditions.py");
}

#[test]
fn matches_cpython_chained_compare_constant_else_fallthrough() {
    for (filename, source) in [
        (
            "chained_compare_literal_else_fallthrough.py",
            r"def choose(worker, files, position):
    if 0 <= position < len(files):
        return (worker, files[position:])
    else:
        42
",
        ),
        (
            "chained_compare_none_else_fallthrough.py",
            r"def choose(worker, files, position):
    if 0 <= position < len(files):
        return (worker, files[position:])
    else:
        None
",
        ),
        (
            "chained_compare_small_return_none_else.py",
            r"def choose(lower, value, upper):
    if lower < value < upper:
        return value
    else:
        None
",
        ),
        (
            "simple_compare_none_else_fallthrough_control.py",
            r"def choose(worker, files, position):
    if position < len(files):
        return (worker, files[position:])
    else:
        None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
