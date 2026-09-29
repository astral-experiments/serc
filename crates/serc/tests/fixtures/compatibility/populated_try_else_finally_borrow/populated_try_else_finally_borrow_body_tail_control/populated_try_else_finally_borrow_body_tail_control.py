async def run(open_stream, manager, progress, observe):
    result = ''
    failure = None
    try:
        stream = await open_stream()
        async with manager:
            value = await stream.read()
        observe()
    except Exception as error:
        failure = str(error)
    else:
        result = extract(value.output)
    finally:
        if progress:
            status = 'ok' if failure is None else 'error'
            await progress.bump(status)
    if failure:
        return '', failure
    return result, None
