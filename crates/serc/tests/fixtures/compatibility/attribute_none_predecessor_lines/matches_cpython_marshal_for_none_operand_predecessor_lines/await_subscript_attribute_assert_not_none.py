async def f(fetch):
    assert (await fetch())[
        "key"
    ].value is not None
