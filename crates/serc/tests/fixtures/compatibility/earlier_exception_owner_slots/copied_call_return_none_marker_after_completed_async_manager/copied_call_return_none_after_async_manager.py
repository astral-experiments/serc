async def collect(resource, payload):
    try:
        async with channel(resource):
            await ready()
        return Result(value=payload)
    except Exception:
        return Result(value=None)
    finally:
        if resource is not None:
            await resource.close()
