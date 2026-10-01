def direct(flag, items, action, other):
    try:
        while False:
            action(items)
        else:
            pass
    finally:
        action(items)
    return other


def handler(flag, items, action, other):
    try:
        action(other)
    except ValueError:
        try:
            while False:
                action(items)
            else:
                pass
        finally:
            action(items)
    return other


def nested(flag, items, action, other):
    try:
        try:
            while False:
                pass
            else:
                pass
        finally:
            action(items)
    finally:
        action(other)
    return other


def manager_tail(flag, items, action, other, manager):
    with manager:
        try:
            while False:
                assert flag
            else:
                pass
        finally:
            action(items)
    return other


async def coroutine(flag, items, action, other):
    try:
        action(other)
    except ValueError:
        try:
            while False:
                pass
            else:
                pass
        finally:
            action(items)
    return other


def generator(flag, items, action, other):
    yield flag
    try:
        action(other)
    except ValueError:
        try:
            while False:
                action(items)
            else:
                pass
        finally:
            action(items)
    return other
