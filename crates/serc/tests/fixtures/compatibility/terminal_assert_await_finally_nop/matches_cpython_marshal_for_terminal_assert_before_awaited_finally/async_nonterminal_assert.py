async def f(value, run, close, recover):
    try:
        assert value
        await run()
    except Exception:
        recover()
    finally:
        await close()
