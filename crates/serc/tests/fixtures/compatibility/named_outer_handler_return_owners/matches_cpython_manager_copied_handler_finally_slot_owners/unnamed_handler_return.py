async def unnamed_handler_return(manager, resource):
    async with manager:
        try:
            await work()
        except Exception:
            report()
            return
        finally:
            if resource is not None:
                await resource.close()
