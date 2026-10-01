def branch(flag, other):
    try:
        if flag:
            value = action()
        else:
            value = other
        assert value
    except Error:
        action()


async def async_branch(flag, other):
    try:
        if flag:
            value = action()
        else:
            value = other
        assert value
    except Error:
        action()


def nested_branch(flag, other):
    try:
        if flag:
            if other:
                value = action()
            else:
                value = other
        assert value
    except Error:
        action()


async def async_nested_branch(flag, other):
    try:
        if flag:
            if other:
                value = action()
            else:
                value = other
        assert value
    except Error:
        action()


def loop(flag, items):
    value = flag
    try:
        for item in items:
            if flag:
                break
            value = action(item)
        assert value
    except Error:
        action()


async def async_loop(flag, items):
    value = flag
    try:
        for item in items:
            if flag:
                break
            value = action(item)
        assert value
    except Error:
        action()
