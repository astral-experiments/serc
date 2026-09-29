async def coroutine_multiple_handlers():
    await source()
    try:
        action()
    except ValueError:
        first()
    except TypeError:
        second()


def generator_multiple_handlers():
    yield source()
    try:
        action()
    except ValueError:
        first()
    except TypeError:
        second()


def with_multiple_handlers(manager):
    with manager:
        try:
            action()
        except ValueError:
            first()
        except TypeError:
            second()


def finally_multiple_handlers():
    try:
        try:
            action()
        except ValueError:
            first()
        except TypeError:
            second()
    finally:
        cleanup()


async def conditional_exception_type():
    await source()
    try:
        action()
    except (FirstError if condition else SecondError):
        recover()


async def single_handler_control():
    await source()
    try:
        action()
    except ValueError:
        recover()


async def tuple_handler_control():
    await source()
    try:
        action()
    except (ValueError, TypeError):
        recover()


async def bare_fallback_control():
    await source()
    try:
        action()
    except ValueError:
        recover()
    except:
        fallback()
