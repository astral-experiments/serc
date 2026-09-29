async def f(value, runner):
    try:
        if value:
            print("skipping")
            print(value, "skipping")
            print("skipping")
            return
        await work()
    except Exception as error:
        print(error)
    finally:
        if runner.manager and runner.manager.session is not None:
            await runner.manager.session.stop()
        await runner.close()
