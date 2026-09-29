async def collect(manager, resource, pending, wait, report, value):
    def retain_resource():
        return resource

    try:
        source = pending
        with manager:
            try:
                for task in source:
                    await wait(task)
                while pending:
                    await wait(pending)
            except Exception:
                for task in pending:
                    task.cancel()
                if pending:
                    await wait(pending)
                raise
    finally:
        await resource.close()
    report(value)
    return value
