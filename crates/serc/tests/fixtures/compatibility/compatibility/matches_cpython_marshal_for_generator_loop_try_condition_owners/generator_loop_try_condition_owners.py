async def coroutine_not_raise(flag, values, step, pause):
    while flag():
        try:
            if not values:
                raise RuntimeError
            step(values)
        except Exception:
            pause()
        await pause()


async def coroutine_attribute_raise(flag, state, step, pause):
    while flag():
        try:
            if not state.ready:
                raise RuntimeError
            step(state)
        except Exception:
            pause()
        await pause()


async def coroutine_call_raise(flag, ready, step, pause):
    while flag():
        try:
            if not ready():
                raise RuntimeError
            step()
        except Exception:
            pause()
        await pause()


async def coroutine_boolean_raise(flag, left, right, step, pause):
    while flag():
        try:
            if left and right:
                raise RuntimeError
            step(left)
        except Exception:
            pause()
        await pause()


async def coroutine_return(flag, values, step, pause):
    while flag():
        try:
            if not values:
                return
            step(values)
        except Exception:
            pause()
        await pause()


async def coroutine_continue(flag, values, step, pause):
    while flag():
        try:
            if not values:
                continue
            step(values)
        except Exception:
            pause()
        await pause()


async def coroutine_break(flag, values, step, pause):
    while flag():
        try:
            if not values:
                break
            step(values)
        except Exception:
            pause()
        await pause()


async def coroutine_effect(flag, values, step, pause):
    while flag():
        try:
            if not values:
                pause()
            step(values)
        except Exception:
            pause()
        await pause()


async def coroutine_body_await(flag, values, step, pause):
    while flag():
        try:
            if values:
                await pause()
            step(values)
        except Exception:
            pause()


async def coroutine_two_handlers(flag, values, step, pause):
    while flag():
        try:
            if not values:
                raise RuntimeError
            step(values)
        except OSError:
            pause()
        except Exception:
            pause()
        await pause()


async def coroutine_prefix_call(flag, values, step, pause):
    while flag():
        try:
            result = step(*values)
            if result:
                return result
        except Exception:
            pause()
        except Exception as error:
            pause()
            raise error


def generator_not_raise(flag, values, step, pause):
    while flag():
        try:
            if not values:
                raise RuntimeError
            step(values)
        except Exception:
            pause()
        yield None


def generator_after_yield(flag, values, step, pause):
    yield None
    while flag():
        try:
            if not values:
                raise RuntimeError
            step(values)
        except Exception:
            pause()


async def comparison_control(flag, value, step, pause):
    while flag():
        try:
            if value < 2:
                raise RuntimeError
            step(value)
        except Exception:
            pause()
        await pause()


async def none_control(flag, value, step, pause):
    while flag():
        try:
            if value is None:
                raise RuntimeError
            step(value)
        except Exception:
            pause()
        await pause()


async def global_callable_control(flag, value, step, pause):
    while flag():
        try:
            if isinstance(value, list):
                step(value)
            step(value)
        except OSError:
            pause()
        await pause()


async def in_try_suspension_control(flag, value, step, pause):
    while flag():
        try:
            await pause()
            if not value:
                raise RuntimeError
            step(value)
        except OSError:
            pause()


async def unpack_prefix_control(flag, values, step, pause):
    while flag():
        try:
            left, right = step(values)
            if left or right:
                pause()
            step(values)
        except Exception:
            pause()
        await pause()


async def finally_control(flag, values, step, pause):
    while flag():
        try:
            if not values:
                raise RuntimeError
            step(values)
        except Exception:
            pause()
        finally:
            step(values)
        await pause()


async def context_manager_control(flag, manager, values, step, pause):
    while flag():
        with manager():
            try:
                if not values:
                    raise RuntimeError
                step(values)
            except Exception:
                pause()
        await pause()


async def iterator_control(values, step, pause):
    for value in values:
        try:
            if not value:
                raise RuntimeError
            step(value)
        except Exception:
            pause()
        await pause()


def synchronous_control(flag, values, step, pause):
    while flag():
        try:
            if not values:
                raise RuntimeError
            step(values)
        except Exception:
            pause()


async def no_loop_control(values, step, pause):
    try:
        if not values:
            raise RuntimeError
        step(values)
    except Exception:
        pause()
    await pause()
