async def run_handler(action, current_task):
    try:
        await action()
    except CancelledError:
        task = current_task()
        if task is not None:
            raise
