async def unnamed_handler_nested_loop_return(manager, second, resource, values):
    async with manager:
        async with second:
            for outer in values:
                for inner in values:
                    try:
                        await work()
                    except Exception:
                        report(outer, inner)
                        return
                    finally:
                        if resource is not None:
                            await resource.close()
