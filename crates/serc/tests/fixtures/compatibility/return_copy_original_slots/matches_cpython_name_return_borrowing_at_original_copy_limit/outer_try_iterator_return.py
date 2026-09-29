async def probe(items, build, report):
    for item in items:
        try:
            try:
                result = build(item)
            except Exception as error:
                report(error)
                continue
            return result
        except Exception as error:
            report(error)
    return None
