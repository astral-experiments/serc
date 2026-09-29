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


def handler_iterator_try(values, callback):
    try:
        return callback()
    except ValueError:
        for value in values:
            try:
                return callback(value)
            except ValueError:
                continue
    return None


def named_handler_nested_iterators(values, callback):
    try:
        return callback()
    except ValueError as error:
        for row in values:
            for value in row:
                try:
                    return callback(value, error)
                except ValueError:
                    continue
    return None


def handler_iterator_literal_return(values, callback):
    try:
        return callback()
    except ValueError:
        for value in values:
            try:
                callback(value)
                return None
            except ValueError:
                continue
    return False


def surrounding_iterator_handler_control(values, callback):
    for value in values:
        try:
            callback(value)
        except ValueError:
            return callback()
    return None
