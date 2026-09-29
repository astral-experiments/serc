async def run(manager, resource):
    async with manager:
        try:
            await work()
        except Exception as error:
            if resource is not None:
                await report(error)
            return f"failed: {error}"
        finally:
            if resource is not None:
                try:
                    await resource.close()
                except Exception as cleanup_error:
                    warn(cleanup_error)
