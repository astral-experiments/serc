async def backfill(limit, fetch, write, report, finish):
    offset = 0
    try:
        while offset < limit:
            page = await fetch()
            if not page:
                break
            await write(page)
            offset += len(page)
            if len(page) < limit:
                report("last page")
                break
    except Exception:
        report("failed")
    finally:
        await finish()
        report("finished")
