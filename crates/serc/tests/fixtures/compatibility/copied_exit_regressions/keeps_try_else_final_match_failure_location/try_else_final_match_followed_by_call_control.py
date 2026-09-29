async def target(items, operate, record):
    total = 0
    async for item in items:
        try:
            outcome = await operate(item)
        except Exception as error:
            outcome = Status.FAILED
            record(error)
        else:
            record(outcome)
            match outcome:
                case Status.CREATED:
                    total += 1
                case Status.REUSED | Status.SKIPPED:
                    total += 2
                case Status.FAILED:
                    raise AssertionError("failure belongs to recovery")
            record("else finished")
        record(outcome)
    return total
