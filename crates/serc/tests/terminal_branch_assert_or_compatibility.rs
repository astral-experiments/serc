#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_terminal_branch_assert_or_success_exits() {
    let source = r#"def terminal_or(enabled, left, right):
    if enabled:
        assert left or right


def terminal_multiline_or(enabled, left, right):
    if enabled:
        prepare()
        assert (
            check(left)
            or check(right)
        ), "missing value"


def terminal_elif_or(enabled, alternate, left, right):
    if enabled:
        prepare()
    elif alternate:
        assert left or right


def terminal_else_or(enabled, left, right):
    if enabled:
        prepare()
    else:
        assert left or right


def terminal_both_branch_or(enabled, left, right, other_left, other_right):
    if enabled:
        assert left or right
    else:
        assert other_left or other_right


def terminal_nested_and_or(enabled, left, middle, right):
    if enabled:
        assert (left and middle) or right


def terminal_nested_call_and_or(enabled, value):
    if enabled:
        assert (value.left() and value.middle()) or value.right()


def terminal_nested_compare_and_or(enabled, left, expected, kwargs, expected_kwargs, length, expected_length):
    if enabled:
        assert left == expected or (
            kwargs == expected_kwargs and length == expected_length
        )


def protected_terminal_or(enabled, left, right):
    try:
        if enabled:
            assert left or right
    except Exception:
        recover()


async def coroutine_terminal_or(enabled, left, right):
    if enabled:
        assert left or right


def generator_terminal_or(enabled, left, right):
    yield enabled
    if enabled:
        assert left or right


def terminal_and_control(enabled, left, right):
    if enabled:
        assert left and right


def nonterminal_or_control(enabled, left, right):
    if enabled:
        assert left or right
    consume()


def unconditional_terminal_or_control(left, right):
    assert left or right


def terminal_simple_assert_control(enabled, value):
    if enabled:
        assert value
"#;

    common::assert_matches_cpython_marshal(source, "terminal_branch_assert_or_success_exits.py");
}
