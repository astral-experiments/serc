async def probe(fetch, remaining):
    collected = []
    cursor = None
    try:
        while True:
            page = await fetch(cursor)
            collected.extend(page.items)
            if remaining is not None:
                remaining -= len(page.items)
            cursor = page.next_cursor
            if cursor is None:
                break
    except Exception as error:
        report(error)
        return []
    return collected
