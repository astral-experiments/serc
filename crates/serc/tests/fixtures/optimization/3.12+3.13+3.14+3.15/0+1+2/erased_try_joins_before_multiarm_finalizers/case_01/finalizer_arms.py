def conditional_finalizer(flag, other):
    try:
        if flag:
            assert other
        else:
            assert flag
    finally:
        if flag:
            action()
        else:
            action()


def chained_finalizer(flag, other):
    try:
        if flag:
            assert other
        else:
            assert flag
    finally:
        if flag:
            action()
        elif other:
            action()
        else:
            action()


async def awaited_finalizer(flag, other):
    try:
        if flag:
            assert other
        else:
            assert flag
    finally:
        if flag:
            await action()
        elif other:
            await action()
        else:
            action()


async def named_awaitable_finalizer(flag, other, first, second):
    try:
        if flag:
            assert other
        else:
            assert flag
    finally:
        if flag:
            await first
        else:
            await second
