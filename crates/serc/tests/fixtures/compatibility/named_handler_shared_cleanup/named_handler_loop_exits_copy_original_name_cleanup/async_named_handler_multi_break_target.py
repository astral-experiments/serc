async def run(items, work, report):
    try:
        await work(None)
    except LookupError as caught:
        report(None)
        for item in items:
            value = await work(item)
            if value is None:
                report(item)
                break
            if value:
                break
            try:
                await work(value)
                break
            except ValueError:
                report(value)
        else:
            await work(None)
    report(None)
