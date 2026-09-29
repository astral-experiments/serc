async def process(count, ready, report, finish):
    if count < 0:
        report("all")
    elif count == 0:
        assert ready
    else:
        report("some")
    await finish()
