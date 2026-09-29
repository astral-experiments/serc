async def run(work, choose, report):
    try:
        await work()
    except* ValueError as group:
        report(group)
        assert len(group.exceptions) == 1
    except* TypeError:
        raise
