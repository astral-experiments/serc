async def run_jobs(config, tasks, cancelled_error, sleep, gather, report):
    reason = 'running'
    try:
        if config.invalid:
            raise ValueError()
        while True:
            stopping = config.stopping
            done = {task for task in tasks if task.done()}
            for task in done:
                tasks.remove(task)
                report(task.result())
            if stopping and not tasks:
                reason = 'expired' if config.expired else 'limit'
                break
            while config.should_launch():
                tasks.add(config.start_task())
            if config.idle:
                await sleep()
    except cancelled_error:
        reason = 'cancelled'
        raise
    except Exception:
        reason = 'error'
        raise
    finally:
        for task in tasks:
            task.cancel()
        if tasks:
            await gather(*tasks, return_exceptions=True)
        report(reason)
    return reason
