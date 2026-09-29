async def named_handler_return(manager, resource):
    async with manager:
        try:
            await work()
        except Exception as exc:
            report(exc)
            return None
        finally:
            if resource is not None:
                await resource.close()
