async def linear(value, action, consume):
    try:
        await action()
    except Exception:
        await action()
    if value:
        consume(value)


async def guarded(flag, value, action, consume):
    try:
        if flag:
            await action()
    except Exception:
        await action()
    if value:
        consume(value)


async def paired_finally(flag, value, action, consume):
    try:
        if flag:
            await action()
        else:
            try:
                await action()
            finally:
                action()
    except Exception:
        await action()
    if value:
        consume(value)


async def populated_exit(flag, value, action, consume):
    try:
        if flag:
            await action()
        else:
            try:
                await action()
            finally:
                action()
        action()
    except Exception:
        await action()
    if value:
        consume(value)
