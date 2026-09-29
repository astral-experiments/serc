async def target(manager, session, report):
    try:
        async with manager:
            await session.run()
    except Exception as error:
        try:
            report(error)
        except Exception:
            pass
    finally:
        report("closing")
        await session.close()
    report("done")
