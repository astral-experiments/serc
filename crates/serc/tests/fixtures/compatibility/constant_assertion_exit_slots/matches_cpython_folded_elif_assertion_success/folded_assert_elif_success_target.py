async def process(count, report, finish):
    if count < 0:
        report("all")
    elif count == 0:
        assert "nothing to do"
    else:
        report("some")
    await finish()
