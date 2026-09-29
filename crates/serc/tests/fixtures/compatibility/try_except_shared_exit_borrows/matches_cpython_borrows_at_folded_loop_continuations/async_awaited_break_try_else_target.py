async def run(state, work, stop, advance, report, limit):
    complete = False
    iteration = 0
    try:
        while True:
            if state.stop:
                break
            if await stop():
                complete = True
                break
            if limit > 0 and iteration >= limit:
                break
            await work()
            iteration += 1
            if await stop():
                complete = True
                break
            if limit > 0 and iteration >= limit:
                break
            if iteration >= limit:
                advance(iteration)
    except Exception as error:
        await report(error)
        raise
    else:
        if not complete:
            complete = await stop()
        status = "done" if complete else "stopped"
        await report(status)
