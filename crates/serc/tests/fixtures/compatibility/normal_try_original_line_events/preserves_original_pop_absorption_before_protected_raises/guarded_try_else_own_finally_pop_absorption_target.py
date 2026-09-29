async def acquire_checked(connect, cleanup, active, key):
    try:
        value = await connect()
    except BaseException:
        raise
    else:
        if key not in active:
            await value.close()
            raise RuntimeError('inactive')
    finally:
        if key in active:
            cleanup(key)
    return value
