async def recover(fetch, report, allow_missing):
    result = None
    try:
        result = await fetch()
    except Exception as error:
        if error.status == 404:
            report(error)
        elif error.status == 403:
            if allow_missing:
                report(error)
            else:
                report(error)
                raise
        else:
            report(error)
            raise error
    return result
