async def example(values):
    try:
        for value in values:
            try:
                if value:
                    try:
                        await operation(value)
                    except TimeoutError:
                        record(value)
                        return
                await consume(value)
            except Exception:
                recover(value)
    except Exception:
        finish()
