async def handle(ready, attempt, recover, finish, close):
    try:
        try:
            result = await attempt()
        except Exception as error:
            result = error
            if ready:
                marker = result
                try:
                    status = await recover(marker)
                except Exception as recovery_error:
                    return recovery_error
                if status != 0:
                    raise RuntimeError('bad status')
        finish(result)
    finally:
        await close()
