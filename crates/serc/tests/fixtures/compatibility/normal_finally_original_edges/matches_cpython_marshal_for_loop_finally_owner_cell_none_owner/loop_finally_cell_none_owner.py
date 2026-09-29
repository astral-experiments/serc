async def f(manager, tasks, progress, writer):
    def capture():
        return progress, writer
    with manager:
        try:
            while tasks:
                await drain(tasks)
        finally:
            progress.close()
            if writer is not None:
                writer.close()
