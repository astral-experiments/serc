async def prior_assert_all(values, ready, tick):
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
