async def run(work, choose, report):
    try:
        await work()
    except* ValueError as group:
        if choose(group):
            report(group)
        else:
            assert len(group.exceptions) == 1
    except* TypeError:
        raise
