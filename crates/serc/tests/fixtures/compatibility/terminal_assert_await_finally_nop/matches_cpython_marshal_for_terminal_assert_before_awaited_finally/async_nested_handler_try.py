async def f(value, close, recover):
    try:
        run()
    except Exception:
        try:
            assert value
        except ValueError:
            recover()
        finally:
            await close()
