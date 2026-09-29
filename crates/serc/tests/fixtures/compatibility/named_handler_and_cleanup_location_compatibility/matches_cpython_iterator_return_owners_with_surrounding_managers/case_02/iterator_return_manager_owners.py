async def f(items, outer, inner, state):
    async with outer:
        with inner:
            try:
                for item in items:
                    try:
                        await work(item)
                        return state.value
                    except Error:
                        recover(item)
            finally:
                finish()
