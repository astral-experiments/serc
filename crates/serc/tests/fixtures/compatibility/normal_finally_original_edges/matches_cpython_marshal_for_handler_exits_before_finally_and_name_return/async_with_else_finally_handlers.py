async def probe(manager, work, report, clock):
    started = clock()
    status = "failed"
    failure = default_reason
    result = None
    try:
        async with manager:
            result = await work()
    except CancelledError:
        status = "cancelled"
        failure = cancelled_reason
        raise
    except TimeoutError:
        failure = timeout_reason
        report(failure.value)
    except CustomError as error:
        failure = error.reason
        report(failure.value)
    except Exception:
        report(failure.value)
    else:
        status = "success"
        failure = None
    finally:
        reason = failure.value if failure is not None else "none"
        tags = [f"result:{status}", f"reason:{reason}"]
        report(tags=tags)
        report((clock() - started) * 1000, tags=tags)
    return result
