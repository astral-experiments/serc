use crate::common;

#[test]
fn matches_cpython_marshal_for_shifted_try_none_condition_owners() {
    let source = r"
def boolop_nonterminal(first, second, lookup, use):
    try:
        value = first or lookup(second)
        if first is not None:
            use(first)
        use(value)
    except ValueError:
        use(second)


def assert_terminal(value, flag, use):
    try:
        assert flag
        if value is None:
            use(value)
    except ValueError as error:
        use(error)


def if_nonterminal(value, flag, use):
    try:
        if flag:
            use(flag)
        if value is not None:
            use(value)
        use(flag)
    except ValueError:
        use(value)


def loop_nonterminal(value, values, use):
    try:
        for item in values:
            use(item)
        if value is None:
            use(value)
        use(values)
    except ValueError:
        use(value)


def nested_with_control(manager, value, flag, use):
    try:
        with manager:
            assert flag
            if value is None:
                use(value)
    except ValueError:
        use(flag)


def deferred_control(value, values, flag, use):
    try:
        callback = lambda item: item if flag else value
        listed = [item for item in values if item is not None]
        generated = (item for item in values if item is not None)
        use(callback, listed, generated)
        if value is None:
            use(value)
    except ValueError:
        use(flag)


def generator_control(value, flag, use):
    yield flag
    try:
        if flag:
            use(flag)
        if value is not None:
            use(value)
    except ValueError:
        use(value)


async def coroutine_control(value, flag, use, ready):
    try:
        assert flag
        if value is None:
            use(value)
        await ready()
    except ValueError:
        use(flag)
";

    common::assert_matches_cpython_marshal(source, "shifted_try_none_condition_owners.py");
}
