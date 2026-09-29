async def handle(ready, attempt, recover, finish, close):
    try:
        try:
            result = await attempt()
        except Exception as error:
            result = error
            if ready:
                marker = result
                status = await recover(marker)
                if status != 0:
                    raise RuntimeError('bad status')
        finish(result)
    finally:
        await close()
