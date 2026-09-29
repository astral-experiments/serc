async def probe(item, build, report):
    try:
        try:
            result = build(item)
        except Exception as error:
            report(error)
            return None
        return result
    except Exception as error:
        report(error)
    return None
