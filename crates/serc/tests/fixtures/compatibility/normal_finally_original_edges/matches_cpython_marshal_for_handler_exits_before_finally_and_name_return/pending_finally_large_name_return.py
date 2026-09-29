async def two_instruction_finally(manager, work, report):
    marker = None
    result = None
    try:
        async with manager:
            result = await work()
    except RuntimeError:
        raise
    except Exception:
        report("failed")
    else:
        marker = "success"
        marker = None
    finally:
        marker = None
    return result


async def call_finally(manager, work, report):
    marker = None
    result = None
    try:
        async with manager:
            result = await work()
    except RuntimeError:
        raise
    except Exception:
        report("failed")
    else:
        marker = "success"
        marker = None
    finally:
        report(marker)
    return result
