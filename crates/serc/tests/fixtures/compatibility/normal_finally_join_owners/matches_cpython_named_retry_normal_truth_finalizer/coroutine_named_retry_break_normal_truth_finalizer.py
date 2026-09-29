async def run(work, retry, items, report, finish):
    error = None
    try:
        await work()
    except ConnectionError as caught:
        error = caught
        if retry:
            for item in items:
                try:
                    await work()
                    error = None
                    return
                except ValueError as caught:
                    error = caught
                    await report(item)
                except Exception as caught:
                    error = caught
                    break
    except Exception as caught:
        error = caught
    finally:
        if error:
            await report(error)
        finish()
