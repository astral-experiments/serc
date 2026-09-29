async def run_handler(action, current_task, terminal_seen):
    try:
        await action()
    except CancelledError:
        task = current_task()
        if not terminal_seen or (task is not None and task.cancelling()):
            raise
