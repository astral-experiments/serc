async def run(values, task, active_error):
    for value in values:
        error = None
        try:
            await work(value)
        except RuntimeError as exc:
            continue
        finally:
            if task is not None:
                try:
                    await task()
                except RuntimeError as exc:
                    error = exc
            if error is not None and active_error is None:
                raise error
