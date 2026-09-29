async def run(work, report, finish):
    error = None
    try:
        await work()
    except Exception as caught:
        error = caught
    finally:
        if error:
            await report(error)
        finish()
