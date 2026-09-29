async def f(manager):
    try:
        await work()
    except Exception as error:
        report(error)
        with manager:
            mark_failed()
        raise
    finally:
        finish()
