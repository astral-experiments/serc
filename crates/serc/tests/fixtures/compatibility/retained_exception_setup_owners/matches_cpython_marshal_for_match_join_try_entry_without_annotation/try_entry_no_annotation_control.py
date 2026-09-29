async def f(source, kind, key, skip):
    container = await source.container(key)
    match kind:
        case 0:
            mode = None
            tag = None
        case 1:
            tag = source.tag(key)
            mode = source.Mode.CHECK if tag is not None else source.Mode.MISSING
        case _:
            raise ValueError(kind)
    updated = False
    limits = source.timeouts(container)
    operation = source.Operation.WRITE
    excluded = source.exclude(operation.name, container.name)
    try:
        if skip:
            transaction = source.empty()
        else:
            transaction = source.start(container.name, key=str(key))
        async with transaction as token:
            with source.report(container, operation) as tracker, source.track(tracker):
                return await source.commit(token, key, tracker, excluded, limits, mode, tag, updated)
    except LookupError:
        return None
