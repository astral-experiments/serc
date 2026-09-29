def bare_call(value):
    try:
        try:
            return primary(value)
        except Exception:
            return fallback(value)
    except Exception:
        return None


def bare_literal(value):
    try:
        prepare(value)
        try:
            validate(value)
        except Exception:
            return None
        return value
    except Exception:
        return False


def named_inner(value):
    try:
        try:
            return primary(value)
        except Exception as error:
            return describe(error)
    except Exception:
        return None


def named_outer(value):
    try:
        try:
            return primary(value)
        except Exception:
            return fallback(value)
    except Exception as error:
        return describe(error)


def conditional_inner(value, enabled):
    try:
        prepared = prepare(value)
        if enabled:
            try:
                return primary(prepared)
            except Exception:
                return fallback(prepared)
        return prepared
    except Exception:
        return None


def preceding_with(manager, value, enabled):
    try:
        with manager:
            prepared = prepare(value)
        if not enabled:
            return None
        try:
            validate(prepared)
        except Exception:
            report(prepared)
            return None
        return primary(prepared)
    except Exception:
        return None


def while_body(value):
    while value:
        try:
            try:
                return primary(value)
            except Exception:
                return fallback(value)
        except Exception:
            value = advance(value)
    return None


async def coroutine(value):
    await prepare(value)
    try:
        try:
            return primary(value)
        except Exception:
            return fallback(value)
    except Exception:
        return None


def generator(value):
    yield value
    try:
        try:
            return primary(value)
        except Exception:
            return fallback(value)
    except Exception:
        return None


async def async_generator(value):
    yield value
    try:
        try:
            primary(value)
            return
        except Exception:
            fallback(value)
            return
    except Exception:
        return


def outer_handler(value):
    try:
        return primary(value)
    except Exception:
        try:
            return secondary(value)
        except Exception:
            return fallback(value)


def outer_handler_conditional_body(value, enabled):
    try:
        return primary(value)
    except Exception as error:
        try:
            prepared = secondary(value)
            if enabled:
                return prepared
        except Exception:
            report(error)
        return fallback(value)


async def coroutine_outer_handler_nested_body(primary, fetch, valid):
    try:
        await primary()
    except RuntimeError as error:
        if not valid(error):
            raise
        try:
            result = await fetch()
            if valid(result):
                return result
        except LookupError:
            raise ValueError() from error
        raise


def outer_finally(value):
    try:
        try:
            return primary(value)
        except Exception:
            return fallback(value)
    finally:
        cleanup(value)
