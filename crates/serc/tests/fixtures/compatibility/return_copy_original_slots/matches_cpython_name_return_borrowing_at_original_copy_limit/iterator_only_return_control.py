async def probe(items, build, report):
    for item in items:
        try:
            result = build(item)
        except Exception as error:
            report(error)
            continue
        return result
    return None
