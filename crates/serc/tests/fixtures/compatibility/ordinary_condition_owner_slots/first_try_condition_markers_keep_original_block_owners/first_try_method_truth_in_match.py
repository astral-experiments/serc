async def target(obj, items):
    async for item in items:
        match item:
            case Event():
                note()
                try:
                    if obj.enabled():
                        await act(item)
                    else:
                        await other(item)
                finally:
                    obj.finish(item)
