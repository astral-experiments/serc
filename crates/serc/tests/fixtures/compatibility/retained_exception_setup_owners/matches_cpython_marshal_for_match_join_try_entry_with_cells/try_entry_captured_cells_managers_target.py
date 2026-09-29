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
        transaction: object
        if skip:
            transaction = source.empty()
        else:
            transaction = source.start(container.name, key=str(key))
        async with transaction as token:
            with source.report(container, operation) as tracker, source.track(tracker):
                def response(value):
                    nonlocal updated
                    updated = value
                result = await source.retry(
                    lambda: source.commit(container, excluded), response
                )
        return result, updated
    except LookupError:
        return None
