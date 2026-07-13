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
