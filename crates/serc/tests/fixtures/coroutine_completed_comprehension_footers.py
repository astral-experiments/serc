async def completed_eager(flag, items, consume):
    if flag:
        out = items
    else:
        await consume(items)
        out = [item for item in items]
    assert out


async def completed_suspending(flag, items, consume):
    if flag:
        out = items
    else:
        await consume(items)
        out = [await consume(item) for item in items]
    assert out


async def outside_prefix(flag, items, consume):
    await consume(items)
    if flag:
        out = items
    else:
        out = [item for item in items]
    assert out


async def tested_arm(flag, items, consume):
    if flag:
        await consume(items)
        out = [item for item in items]
    else:
        out = items
    assert out
