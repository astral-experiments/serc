def make_worker(manager, finish):
    async def worker(items, fail_fast):
        try:
            async with manager:
                raw = await fetch()
            for item in items:
                consume(item, raw)
        except Exception as error:
            if fail_fast:
                report(error)
            for item in items:
                recover(item, error)
        finally:
            finish(len(items))

    return worker
