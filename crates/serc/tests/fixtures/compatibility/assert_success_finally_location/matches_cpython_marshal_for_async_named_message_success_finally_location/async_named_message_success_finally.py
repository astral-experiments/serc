async def f(value, allowed, close, recover):
    try:
        assert isinstance(value, str) and value in allowed, f'Unexpected: {value!r}'
    except Exception as exc:
        recover(exc)
        raise
    finally:
        await close()
