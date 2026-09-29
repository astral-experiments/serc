async def close_after_handler_return(manager, acquire):
    async with manager:
        resource = None

        async def sample():
            return resource

        try:
            resource = await acquire()
            result = await sample()
        except Exception:
            return None
        finally:
            if resource is not None:
                await resource.close()

    return result
