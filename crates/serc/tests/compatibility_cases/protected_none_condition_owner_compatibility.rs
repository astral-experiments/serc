use crate::common;

#[test]
fn matches_cpython_marshal_for_protected_none_condition_owners() {
    let source = r"
def repeated_with_conditions(manager, first, second, use):
    with manager:
        if first is not None:
            use(first)
        if second is not None:
            use(second)


def shifted_with_conditions(manager, flag, value, use):
    with manager:
        assert flag
        if value is not None:
            use(value)
        if flag:
            use(value)
        if value is None:
            use(None)


def repeated_try_assertions(holder, use):
    try:
        assert holder.value is not None
        assert holder.value.name is not None
        use(holder.value.name)
    except ValueError:
        pass


def shifted_try_assertions(flag, values, use):
    try:
        assert flag
        for value in values:
            assert value is not None
            use(value)
    except ValueError:
        pass


def repeated_finally_assertions(holder, use):
    try:
        assert holder.value is None
        assert holder.value.name is None
        use(holder.value)
    finally:
        use(holder)


def iterator_with_control(manager, values, use):
    with manager:
        for value in values:
            if value is not None:
                use(value)


async def async_with_control(manager, first, second, use):
    async with manager:
        if first is not None:
            use(first)
        if second is not None:
            use(second)


def generator_try_control(holder, use):
    try:
        assert holder.value is not None
        assert holder.value.name is not None
        yield holder.value.name
    except ValueError:
        use(holder)


async def coroutine_try_control(holder, use):
    try:
        assert holder.value is not None
        assert holder.value.name is not None
        await use(holder.value.name)
    except ValueError:
        pass
";

    common::assert_matches_cpython_marshal(source, "protected_none_condition_owners.py");
}
