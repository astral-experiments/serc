async def f(settings, work, pause):
    i = 0
    while True:
        i += 1
        try:
            await work()
        except Exception as error:
            record(error)
        if 0 < settings.iteration <= i:
            return
        if settings.sleep_time > 0:
            await pause(settings.sleep_time)
