async def probe(manager, work, report):
    status = "failed"
    failure = None
    result = None
    try:
        async with manager:
            result = await work()
    except RuntimeError:
        raise
    except Exception:
        report("failed")
    else:
        status = "success"
        failure = None
    finally:
        reason = failure.value if failure is not None else "none"
        report(status, reason)
    return result
