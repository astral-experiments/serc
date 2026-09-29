def collect(source, accept, report):
    found = False
    values = []
    try:
        if source.items is not None:
            for item in source.items:
                content = getattr(item, "content", None)
                if accept(content):
                    found = True
                    report(content)
                    break
    except Exception:
        found = False
    report(found, values)
    if found and values:
        return values
    return []
