async def emit_rows(flag, manager, fetch, emit):
    async with manager:
        rows = await fetch()
    if flag:
        for name, value in rows:
            if value:
                emit(name, value)
    else:
        for name, value in rows:
            emit(name, value)
