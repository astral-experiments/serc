async def target(flag, state, item, stage, metrics):
    try:
        if flag:
            one()
            return item
        return item
    except Exception:
        raise
    finally:
        failure = stage if state in {"cancelled", "exception", "unknown"} else None
        metrics.record(failure)
