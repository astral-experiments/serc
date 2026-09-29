async def final_branch(value, enabled, consume):
    await value.wait()
    if enabled:
        assert value.ready()
        consume(value)


async def final_branch_assert_only(value, enabled):
    await value.wait()
    if enabled:
        assert value.ready()


async def final_branch_two_asserts(value, enabled):
    await value.wait()
    if enabled:
        assert value.ready()
        assert value.valid()


async def final_branch_else(value, enabled, consume):
    await value.wait()
    if enabled:
        assert value.ready()
        consume(value)
    else:
        consume(enabled)


async def protected_body(value, consume):
    await value.wait()
    try:
        assert value.ready()
        consume(value)
    except RuntimeError:
        return None


async def protected_finally(value, consume):
    await value.wait()
    try:
        assert value.ready()
        consume(value)
    finally:
        consume(value)
    return None


async def before_suspension_control(value, enabled):
    if enabled:
        assert value.ready()
    await value.wait()


async def top_level_control(value):
    await value.wait()
    assert value.ready()


async def message_control(value, enabled, consume):
    await value.wait()
    if enabled:
        assert value.ready(), "not ready"
    consume(value)


async def argument_control(value, enabled, consume):
    await value.wait()
    if enabled:
        assert value.ready(enabled)
    consume(value)


async def keyword_control(value, enabled, consume):
    await value.wait()
    if enabled:
        assert value.ready(enabled=enabled)
    consume(value)


async def nested_attribute_control(value, enabled, consume):
    await value.wait()
    if enabled:
        assert value.state.ready()
    consume(value)


async def name_call_control(value, enabled, ready, consume):
    await value.wait()
    if enabled:
        assert ready()
    consume(value)


async def with_control(value, enabled, manager):
    await value.wait()
    with manager:
        if enabled:
            assert value.ready()


async def handler_control(value):
    await value.wait()
    try:
        raise RuntimeError
    except RuntimeError:
        assert value.ready()


def synchronous_control(value, enabled):
    if enabled:
        assert value.ready()
