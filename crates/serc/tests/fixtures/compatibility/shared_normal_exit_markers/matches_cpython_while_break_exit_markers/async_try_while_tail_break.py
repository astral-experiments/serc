async def f(ready, step, result):
    try:
        while True:
            await step()
            if not ready():
                break
    except RuntimeError as error:
        recover(error)
        raise error
    return result
