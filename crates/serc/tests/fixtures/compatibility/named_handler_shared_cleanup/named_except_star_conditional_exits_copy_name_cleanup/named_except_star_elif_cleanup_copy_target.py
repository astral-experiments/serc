async def run(work, choose, report):
    try:
        await work()
    except* ValueError as group:
        if choose(group, 0):
            report(group, 0)
        elif choose(group, 1):
            report(group, 1)
        else:
            assert len(group.exceptions) == 1
            report(group, 2)
    except* TypeError:
        raise
