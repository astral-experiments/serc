use crate::common;

#[test]
fn matches_cpython_marshal_for_coroutine_pre_suspension_dispatch_owners() {
    let source = r#"async def prior_assert_all(values, ready, tick):
    assert ready
    assert all(value.ready for value in values)
    await tick()


async def prior_assert_any(values, ready, tick):
    assert ready
    assert any(value.ready for value in values)
    await tick()


async def prior_assert_not_any(values, ready, tick):
    assert ready
    assert not any(value.ready for value in values), "unexpected"
    await tick()


async def prior_if_all(values, flag, prepare, tick):
    if flag:
        prepare(values)
    assert all(value.ready for value in values)
    await tick()


async def prior_if_any(values, flag, prepare, tick):
    if flag:
        prepare(values)
    assert any(value.ready for value in values)
    await tick()


async def prior_if_not_any(values, flag, prepare, tick):
    if flag:
        prepare(values)
    assert not any(value.ready for value in values), "unexpected"
    await tick()


async def nested_if_filtered(values, flag, tick):
    if flag:
        assert any(value.ready for value in values if value.enabled)
    await tick()


async def prior_assert_multigen(entries, ready, tick):
    assert ready
    assert any(value.ready for entry in entries for value in entry.values)
    await tick()


async def first_assert_any(values, tick):
    assert any(value.ready for value in values)
    await tick()


async def prior_assignment_any(values, tick):
    copied = values
    assert any(value.ready for value in copied)
    await tick()
"#;
    common::assert_matches_cpython_marshal(source, "coroutine_pre_suspension_dispatch_owners.py");
}

#[test]
fn matches_cpython_marshal_for_coroutine_pre_suspension_branch_dispatch_owners() {
    let source = r"async def or_assignment(values, flag):
    result = flag or any(value.ready for value in values)
    return result


async def and_assignment(values, flag):
    result = flag and all(value.ready for value in values)
    return result


async def prior_condition(values, flag):
    if flag:
        raise ValueError
    return any(value.ready for value in values)


async def prior_boolean(values, flag, other):
    initial = flag and other
    return initial or any(value.ready for value in values)


async def condition(values, flag):
    if flag or any(value.ready for value in values):
        return True
    return False


async def argument(values, flag, consume):
    return consume(flag, flag or all(value.ready for value in values))


async def conditional(values, flag):
    return any(value.ready for value in values) if flag else True


async def shadowed_callable(values, flag, any):
    return flag or any(value.ready for value in values)


async def awaited_iterable(values, flag):
    return flag or any(value.ready for value in await values())


async def protected_branch(values, flag):
    try:
        return flag or any(value.ready for value in values)
    except ValueError:
        return False


async def first_call_control(values):
    return any(value.ready for value in values)


async def first_operand_control(values, flag):
    return all(value.ready for value in values) and flag
";
    common::assert_matches_cpython_marshal(
        source,
        "coroutine_pre_suspension_branch_dispatch_owners.py",
    );
}
