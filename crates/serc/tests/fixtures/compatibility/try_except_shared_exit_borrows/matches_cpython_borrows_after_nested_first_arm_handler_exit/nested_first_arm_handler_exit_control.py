async def f(context, choose, load, report):
    result = None
    try:
        value = await load(context)
        result = await load(context, value=value)
    except Exception as error:
        if not await choose(context):
            status = await load(context)
            if status and context.active:
                report(context, status)
                value = None
                result = load(context)
            else:
                report(context, status)
                result = load(context)
        else:
            report(context)
            value = None
            result = load(context)
    enabled = choose(context)
    status = await load(context, value=value)
    result.status = status
    if enabled:
        result.value = await load(context)
    report(context, result)
    return result
