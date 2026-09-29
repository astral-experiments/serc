async def target(items, operate, record):
    async for item in items:
        try:
            outcome = await operate(item)
        except Exception as error:
            record(error)
        else:
            match outcome:
                case Status.OK:
                    record(outcome)
                case Status.FAILED:
                    raise AssertionError("failed")
        record(outcome)
    return None
