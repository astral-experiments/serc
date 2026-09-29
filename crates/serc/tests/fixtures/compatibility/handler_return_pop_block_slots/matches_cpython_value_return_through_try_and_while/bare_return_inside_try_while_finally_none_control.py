async def cycle(metrics, events, watcher, completed=False, reason="end"):
    count = 0
    try:
        while True:
            event = await events.next()
            if event is None:
                return
            count += 1
            if event.done:
                completed = True
                reason = None
                return
    except Cancelled:
        reason = None
        raise
    except Exception:
        reason = "error"
        raise
    finally:
        if (
            metrics is not None
            and not completed
            and reason is not None
        ):
            metrics.finish(count, reason=reason)
        watcher.stop()
        if metrics is not None:
            metrics.flush()
