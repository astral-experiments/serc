async def f(fetch):
    assert (
        await fetch()
    ).value is None
