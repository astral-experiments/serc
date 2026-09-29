async def f(value, allowed, outer, inner, close):
    async with outer:
        async with inner:
            try:
                assert isinstance(value, str) and value in allowed, f'Unexpected: {value!r}'
            except Exception as exc:
                raise ValueError() from exc
            finally:
                await close()
