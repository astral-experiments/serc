def normal_copy(flag, items, action, other):
    try:
        action(other)
    except ValueError:
        try:
            while False:
                pass
        finally:
            action(items)
    return other


async def coroutine_copy(flag, items, action, other):
    try:
        action(other)
    except ValueError:
        try:
            while False:
                pass
        finally:
            action(items)
    return other


def generator_copy(flag, items, action, other):
    yield flag
    try:
        action(other)
    except ValueError:
        try:
            while False:
                pass
        finally:
            action(items)
    return other


def named_handler(items, action, other):
    try:
        action(other)
    except ValueError as error:
        try:
            while False:
                pass
        finally:
            action(items)
    return other


def normal_path_stays_strict(items, action, other):
    while False:
        pass
    try:
        action(other)
    except ValueError:
        try:
            while False:
                pass
        finally:
            action(items)
    return other


def plain_pass_control(items, action, other):
    try:
        action(other)
    except ValueError:
        try:
            pass
        finally:
            action(items)
    return other


def call_tail_control(items, action, other):
    try:
        action(other)
    except ValueError:
        try:
            while False:
                pass
        finally:
            action(items)
    return action(other)


def manager_control(items, action, other, manager):
    with manager:
        try:
            action(other)
        except ValueError:
            try:
                while False:
                    pass
            finally:
                action(items)
        return other
