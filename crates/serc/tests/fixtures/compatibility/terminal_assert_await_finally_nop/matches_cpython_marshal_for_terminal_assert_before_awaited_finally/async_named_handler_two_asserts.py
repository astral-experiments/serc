async def f(run, close, recover):
    result, output = await run()
    try:
        assert result == 0
        assert output == b'ok'
    except Exception as exc:
        recover(exc)
    finally:
        await close()
