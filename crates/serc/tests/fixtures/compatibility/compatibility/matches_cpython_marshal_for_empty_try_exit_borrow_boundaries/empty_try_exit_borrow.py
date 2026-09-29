def fallthrough_if(value, flag):
    try:
        if flag:
            consume()
    except Exception:
        pass
    consume(value)
    return value


def terminal_if(value, flag):
    try:
        if flag:
            return value
    except Exception:
        pass
    consume(value)
    return value


def followed_in_try(value, flag):
    try:
        if flag:
            consume()
        other()
    except Exception:
        pass
    consume(value)
    return value


def plain_try(value):
    try:
        consume()
    except Exception:
        pass
    consume(value)
    return value


def shared_outer_join(value, flag, outer):
    if outer:
        try:
            if flag:
                consume()
        except Exception:
            pass
    consume(value)
    return value


def unthreaded_nested_if(value, flag, inner):
    try:
        if flag:
            if inner:
                return value
            else:
                consume()
    except Exception:
        pass
    consume(value)
    return value


def fallthrough_if_in_else(value, flag):
    try:
        consume()
    except Exception:
        pass
    else:
        if flag:
            consume()
    consume(value)
    return value


def complete_if_in_else(value, flag):
    try:
        consume()
    except Exception:
        pass
    else:
        if flag:
            consume()
        else:
            recover()
    consume(value)
    return value
