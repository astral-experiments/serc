async def probe():
    before()
    try:
        assert await check()
    except Error:
        assert True
    assert False
