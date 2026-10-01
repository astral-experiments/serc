async def completed_await(outer, flag, other, action):
    if outer:
        await action(other)
        if flag:
            action(flag)
        else:
            action(other)
        assert flag


async def completed_manager(outer, flag, other, manager, action):
    if outer:
        with manager:
            action(other)
        if flag:
            value = flag
        else:
            value = other
        assert flag


async def completed_condition(outer, flag, other, action):
    if outer:
        if other:
            action(other)
        if flag:
            action(flag)
        else:
            action(other)
        assert flag


async def completed_handler(outer, flag, other, action):
    if outer:
        try:
            action(other)
        except ValueError:
            action(flag)
        if flag:
            action(flag)
        else:
            action(other)
        assert flag


async def awaiting_arms(outer, flag, other, action):
    if outer:
        if flag:
            value = await action(flag)
        else:
            value = await action(other)
        assert flag
