async def two_handlers_reraise(manager, action):
    try:
        async with manager():
            await action()
    except RuntimeError:
        raise
    except Exception as error:
        raise ValueError() from error


async def two_handlers_named(manager, action):
    try:
        async with manager():
            await action()
    except TimeoutError as error:
        action(error)
        raise RuntimeError() from error
    except Exception as error:
        action(error)
        raise ValueError() from error


async def two_handlers_bare(manager, action):
    try:
        async with manager():
            await action()
    except RuntimeError:
        raise
    except Exception:
        raise


async def one_handler_named(manager, action):
    try:
        async with manager():
            await action()
    except Exception as error:
        raise ValueError() from error


async def one_handler_bare(manager, action):
    try:
        async with manager():
            await action()
    except Exception:
        raise


async def two_handlers_assignment(manager, action):
    try:
        async with manager() as value:
            result = await action(value)
    except RuntimeError:
        raise
    except Exception as error:
        raise ValueError() from error
    return result


async def two_handlers_two_managers(manager, action):
    try:
        async with manager(), manager():
            await action()
    except RuntimeError:
        raise
    except Exception as error:
        raise ValueError() from error


async def two_handlers_nested_managers(manager, action):
    try:
        async with manager():
            async with manager():
                await action()
    except RuntimeError:
        raise
    except Exception as error:
        raise ValueError() from error


async def two_handlers_two_awaits(manager, action):
    try:
        async with manager():
            await action()
            await action()
    except RuntimeError:
        raise
    except Exception as error:
        raise ValueError() from error


async def two_handlers_action_after(manager, action):
    try:
        async with manager():
            await action()
        action()
    except RuntimeError:
        raise
    except Exception as error:
        raise ValueError() from error


async def two_handlers_post_try_action(manager, action):
    try:
        async with manager():
            await action()
    except RuntimeError:
        raise
    except Exception as error:
        raise ValueError() from error
    action()


async def two_handlers_post_try_action_return(manager, action):
    try:
        async with manager():
            await action()
    except RuntimeError:
        raise
    except Exception as error:
        raise ValueError() from error
    action()
    return 1


async def one_handler_post_try_action(manager, action):
    try:
        async with manager():
            await action()
    except Exception as error:
        raise ValueError() from error
    action()


async def bare_handler_post_try_action(manager, action):
    try:
        async with manager():
            await action()
    except Exception:
        raise
    action()


async def handler_post_try_await(manager, action):
    try:
        async with manager():
            await action()
    except Exception as error:
        raise ValueError() from error
    await action()


async def two_handlers_try_after(manager, action, value):
    try:
        async with manager():
            result = await action()
    except RuntimeError:
        raise
    except Exception as error:
        raise ValueError() from error
    try:
        if value != 0:
            raise RuntimeError()
        return result
    except RuntimeError:
        raise
    except Exception as error:
        raise ValueError() from error


async def body_suffix_await_control(manager, action):
    try:
        async with manager():
            await action()
        await action()
    except RuntimeError:
        raise
    action()


async def try_else_control(manager, action):
    try:
        async with manager():
            await action()
    except RuntimeError:
        raise
    else:
        action()
    action()


async def try_finally_control(manager, action):
    try:
        async with manager():
            await action()
    finally:
        action()
    action()


async def post_return_control(manager, action):
    try:
        async with manager():
            await action()
    except RuntimeError:
        raise
    return 1


async def post_raise_control(manager, action):
    try:
        async with manager():
            await action()
    except RuntimeError:
        raise
    raise ValueError()


async def plain_try_two_handlers(action):
    try:
        await action()
    except RuntimeError:
        raise
    except Exception as error:
        raise ValueError() from error


async def sync_with_two_handlers(manager, action):
    try:
        with manager():
            await action()
    except RuntimeError:
        raise
    except Exception as error:
        raise ValueError() from error


def synchronous_control(manager, action):
    try:
        with manager():
            action()
    except RuntimeError:
        raise
    except Exception as error:
        raise ValueError() from error
