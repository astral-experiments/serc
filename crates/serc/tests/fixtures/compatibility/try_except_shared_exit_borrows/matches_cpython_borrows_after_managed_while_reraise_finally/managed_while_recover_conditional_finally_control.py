async def collect(outer, manager, resource, pending, wait, report):
    async with outer:
        try:
            with manager:
                try:
                    while pending:
                        completed = await wait(pending)
                        for task in completed:
                            report(task)
                except Exception:
                    for task in pending:
                        task.cancel()
                    await wait(pending)
                    report(pending)
        finally:
            if resource is not None:
                resource.close()
    report(resource)
