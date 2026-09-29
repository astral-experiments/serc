async def retry(check, work, recover, finish, value):
    while True:
        try:
            value = await work(value)
            if check(value):
                return
            else:
                break
        except ValueError:
            if not await recover(value):
                raise
    finish(value)
    return value
