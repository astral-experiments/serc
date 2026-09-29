async def prepare(mode, phases, task, measure, tracer, cleanup, record, now):
    try:
        if mode is phases.ACTIVE:
            await task
        else:
            await task
    except InterruptedError:
        record("interrupted")
        await cleanup()
        raise
    except Exception:
        record("failed")
        await cleanup()
        raise
    finally:
        if mode is phases.ACTIVE:
            finished = now()
            record(finished)
