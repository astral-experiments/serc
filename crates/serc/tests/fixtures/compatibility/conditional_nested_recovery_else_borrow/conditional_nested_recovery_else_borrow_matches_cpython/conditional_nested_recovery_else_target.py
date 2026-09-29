async def collect(fetch, recover, prefer, fallback, report):
    try:
        values = await fetch()
        if prefer and fallback:
            prior = values
            try:
                extras = await recover()
            except Exception:
                if not prior:
                    raise
                report()
                values = prior
            else:
                values = [*prior, *extras]
    except Exception:
        report()
        return {}

    grouped = {}
    for value in values:
        grouped.setdefault(value.key, []).append(value)
    return grouped
