def outer_try_iterator(values):
    try:
        for value in values:
            try:
                validate(value)
            except Exception:
                return fallback(value)
    except Exception:
        return None
    return True


def iterator_outer_try(values):
    for value in values:
        try:
            try:
                return primary(value)
            except Exception:
                return fallback(value)
        except Exception:
            continue
    return None


def named_inner(values):
    try:
        for value in values:
            try:
                validate(value)
            except Exception as error:
                return describe(error)
    except Exception:
        return None
    return True


def named_outer(values):
    try:
        for value in values:
            try:
                validate(value)
            except Exception:
                return fallback(value)
    except Exception as error:
        return describe(error)
    return True


def conditional_handler(values, enabled):
    try:
        for value in values:
            try:
                validate(value)
            except Exception:
                if enabled:
                    return fallback(value)
    except Exception:
        return None
    return True


def nested_iterator(values):
    try:
        for row in values:
            for value in row:
                try:
                    validate(value)
                except Exception:
                    return fallback(value)
    except Exception:
        return None
    return True


def no_outer_try_control(values):
    for value in values:
        try:
            validate(value)
        except Exception:
            return fallback(value)
    return None


def outer_handler_control(value, enabled):
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


async def coroutine(values):
    await prepare(values)
    try:
        for value in values:
            try:
                validate(value)
            except Exception:
                return fallback(value)
    except Exception:
        return None
    return True


def generator(values):
    yield None
    try:
        for value in values:
            try:
                validate(value)
            except Exception:
                return fallback(value)
    except Exception:
        return None
    return True
