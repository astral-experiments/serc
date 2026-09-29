async def named_handler_nested_manager_return(manager, lock, resource):
    async with manager:
        try:
            await work()
        except Exception as exc:
            if lock is not None:
                async with lock:
                    report(exc)
            return None
        finally:
            if resource is not None:
                await resource.close()
