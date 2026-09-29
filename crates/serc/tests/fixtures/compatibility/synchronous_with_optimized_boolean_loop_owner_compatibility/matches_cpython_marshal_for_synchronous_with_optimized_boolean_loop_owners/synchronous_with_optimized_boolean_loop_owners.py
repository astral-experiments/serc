def sync_assert_all(manager, values):
    with manager:
        assert all(value.ready for value in values)


def sync_assert_any(manager, values):
    with manager:
        assert any(value.ready for value in values)


def sync_assert_not_any(manager, values):
    with manager:
        assert not any(value.ready for value in values)


def sync_filtered(manager, values):
    with manager:
        assert all(value.ready for value in values if value)


def sync_nested_with(manager, nested, values):
    with manager:
        with nested:
            assert any(value.ready for value in values)


def sync_try_body(manager, values, use):
    with manager:
        try:
            assert any(value.ready for value in values)
        except Exception:
            use()


def sync_try_handler(manager, values, use):
    with manager:
        try:
            use()
        except Exception:
            assert all(value.ready for value in values)


def sync_try_finally_body(manager, values, use):
    with manager:
        try:
            assert any(value.ready for value in values)
        finally:
            use()


def sync_try_finally_normal(manager, values, use):
    with manager:
        try:
            use()
        finally:
            assert all(value.ready for value in values)


async def coroutine_preawait_assert(manager, values, tick):
    with manager:
        assert any(value.ready for value in values)
        await tick()


def generator_initial_assert(manager, values):
    with manager:
        assert any(value.ready for value in values)
        yield values


def generator_resumed_assert(manager, values):
    with manager:
        yield values
        assert any(value.ready for value in values)


async def async_generator_initial_assert(manager, values):
    with manager:
        assert any(value.ready for value in values)
        yield values
