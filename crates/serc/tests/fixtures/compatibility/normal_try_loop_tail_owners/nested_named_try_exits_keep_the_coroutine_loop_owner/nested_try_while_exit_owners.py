async def worker():
    while True:
        try:
            item = claim()
            if item is None:
                await pause()
                continue
            context = build(item)
            try:
                await execute(context)
            except Exception as error:
                report(error)
                update(item)
        except CancelledError:
            raise
        except Exception:
            report_failure()
