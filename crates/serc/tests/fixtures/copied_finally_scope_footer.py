def conditional_cleanup(flag, other):
    try:
        if flag:
            assert other
    finally:
        if flag:
            action()


def completed_manager_cleanup(flag, other, manager):
    try:
        with manager:
            action()
        if flag:
            assert other
        elif other:
            assert flag
    finally:
        if flag:
            action()
            action()


async def awaited_cleanup(flag, other):
    try:
        await action()
        if flag:
            assert other
        else:
            assert flag
        if other:
            assert flag
        elif flag:
            assert other
    finally:
        if flag:
            action()
            await action()
