async def target(obj, items, stats):
    async for item in items:
        match item:
            case Event():
                note("done")
                try:
                    if obj.enabled():
                        await obj.act(item)
                    else:
                        await obj.other(item)
                finally:
                    obj.finish(item)
            case _:
                obj.ignore(item)
