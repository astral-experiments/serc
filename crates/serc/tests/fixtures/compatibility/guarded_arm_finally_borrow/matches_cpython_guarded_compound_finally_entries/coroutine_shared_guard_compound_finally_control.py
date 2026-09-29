async def f(flag, value, closer, first, second, task):
    try:
        if flag:
            return
        value = await task.run()
        if flag and value is not None:
            task.log(value)
            try:
                if flag:
                    await task.work(value)
            finally:
                task.record(value)
    except ValueError:
        task.cancelled()
        raise
    except Exception:
        task.failed()
    finally:
        try:
            if closer is not None:
                try:
                    async with task.timeout():
                        await closer.close()
                except Exception:
                    task.failed()
        finally:
            if first is not None:
                try:
                    await first.close()
                except Exception:
                    task.failed()
            if second is not None:
                try:
                    await second.close()
                except Exception:
                    task.failed()
