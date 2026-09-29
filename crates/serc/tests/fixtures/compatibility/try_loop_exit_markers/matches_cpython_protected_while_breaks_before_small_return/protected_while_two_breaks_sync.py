def probe(fetch):
    collected = []
    cursor = None
    try:
        while True:
            page = fetch(cursor)
            if page.items:
                collected.extend(page.items)
                if page.has_more:
                    cursor = page.next_cursor
                else:
                    break
            else:
                break
    except Exception as error:
        report(error)
        return []
    return collected
