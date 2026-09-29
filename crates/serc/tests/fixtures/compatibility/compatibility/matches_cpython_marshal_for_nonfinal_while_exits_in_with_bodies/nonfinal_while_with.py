def final_sync(manager, flag):
    with manager:
        while flag:
            step()
    outside()


def followed_sync(manager, flag):
    with manager:
        while flag:
            step()
        inside()
    outside()


def nested_followed_sync(manager, flag, outer):
    with manager:
        if outer:
            while flag:
                step()
            inside()
    outside()


def loop_else_sync(manager, flag):
    with manager:
        while flag:
            step()
        else:
            fallback()
        inside()
    outside()


async def followed_async(manager, flag):
    async with manager:
        while flag:
            step()
        inside()
    outside()
