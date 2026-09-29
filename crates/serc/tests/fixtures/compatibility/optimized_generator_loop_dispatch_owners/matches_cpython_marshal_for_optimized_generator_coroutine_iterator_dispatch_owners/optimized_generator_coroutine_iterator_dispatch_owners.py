async def after_await(values, tick):
    for entry in values:
        await tick()
        found = any(value for value in entry)
        use(found)


async def after_assert(values, ready):
    for entry in values:
        assert ready
        found = all(value for value in entry)
        use(found)


async def iterator_entry_control(values, tick):
    await tick()
    for entry in values:
        if any(value for value in entry):
            break
        await tick()
