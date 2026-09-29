async def f(manager, items, work, flag):
    async with manager:
        for item in items:
            try:
                await work(item)
            finally:
                if flag:
                    value = work()
                    count = value / 2
                    try:
                        assert count == item
                    except AssertionError:
                        pass
                    work(count)
