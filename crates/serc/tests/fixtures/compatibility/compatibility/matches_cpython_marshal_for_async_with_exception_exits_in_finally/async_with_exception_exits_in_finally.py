async def tuple_target(manager, close, value):
    try:
        async with manager() as (left, right):
            result = await right(value)
    except Error:
        log()
        raise
    finally:
        await close()
    return result


async def sync_finalizer(manager, close, value):
    try:
        async with manager() as callback:
            result = callback(value)
    except Error:
        raise
    finally:
        close()
    return result


async def handler_pass(manager, close, value):
    try:
        async with manager() as callback:
            result = await callback(value)
    except Error:
        pass
    finally:
        await close()
    return result


async def nested_try(manager, close, value):
    try:
        try:
            async with manager() as callback:
                result = await callback(value)
        except OtherError:
            raise
    except Error:
        raise
    finally:
        await close()
    return result


async def preceded_and_pass_body(manager, close):
    try:
        log()
        async with manager():
            pass
    except Error:
        raise
    finally:
        await close()
    return True


async def no_handler_control(manager, close, value):
    try:
        async with manager() as callback:
            result = await callback(value)
    finally:
        await close()
    return result


async def following_statement_control(manager, close, value):
    try:
        async with manager:
            result = await use(value)
        result = convert(result)
        return result
    except Error:
        raise
    finally:
        await close()


async def nested_manager_control(first, second, close, value):
    try:
        async with first() as left:
            async with second(left) as callback:
                result = await callback(value)
        use(result)
        return result
    except Error:
        raise
    finally:
        await close()


async def multiple_manager_control(first, second, close, value):
    try:
        async with first(), second() as callback:
            result = await callback(value)
        use(result)
        return result
    except Error:
        raise
    finally:
        await close()
