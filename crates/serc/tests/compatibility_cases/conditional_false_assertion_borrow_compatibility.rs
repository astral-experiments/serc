use crate::common;

#[test]
fn matches_cpython_marshal_for_conditional_false_assertion_borrows() {
    let source = r#"def percent_message(flag, module_name):
    if not flag:
        assert False, "Module %s was not patched" % module_name
    consume(flag)


def fstring_message(items, names):
    matched = [item for item in items if item in names]
    if len(matched) != len(names):
        assert False, f"Items {names} not found"
    return sorted(matched)


def name_message(flag, message):
    if flag:
        assert False, message
    consume(flag)


def zero_message(flag, message):
    if flag:
        assert 0, f"missing {message}"
    consume(flag)


def elif_message(first, second, message):
    if first:
        consume(first)
    elif second:
        assert False, message
    consume(second)


def leading_expression_control(flag, message):
    if flag:
        consume(message)
        assert False, message
    consume(flag)


def else_control(flag, message):
    if flag:
        consume(flag)
    else:
        assert False, message


def standalone_control(message):
    assert False, message


def dynamic_control(flag, condition, message):
    if flag:
        assert condition, message
    consume(flag)


def multiline_false_control(flag, message):
    if flag:
        assert (
            False
        ), f"failed {message}"
    consume(message)


def parenthesized_false_message(flag, message):
    if flag:
        assert (False), f"failed {message}"
    consume(message)


def closing_paren_message(flag, message):
    if flag:
        assert (False
        ), f"failed {message}"
    consume(message)


def multiline_message(flag, message):
    if flag:
        assert False, (
            f"failed {message}"
        )
    consume(message)


def multiline_zero_control(flag, message):
    if flag:
        assert (
            0
        ), f"failed {message}"
    consume(message)


def multiline_none_control(flag, message):
    if flag:
        assert (
            None
        ), f"failed {message}"
    consume(message)
"#;
    common::assert_matches_cpython_marshal(source, "conditional_false_assertion_borrows.py");
}
