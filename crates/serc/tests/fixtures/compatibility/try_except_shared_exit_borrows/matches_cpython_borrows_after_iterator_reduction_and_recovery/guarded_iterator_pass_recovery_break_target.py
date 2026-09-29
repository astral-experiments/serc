def collect(source, accept, report):
    found = False
    values = []
    try:
        if source.items is not None:
            for item in source.items:
                content = getattr(item, "content", None)
                if accept(content):
                    found = True
                    try:
                        key = getattr(content, "key", None)
                        value = getattr(content, "value", None)
                        if isinstance(key, str) and isinstance(value, str) and value:
                            values.append((key, value))
                    except Exception:
                        pass
                    break
    except Exception:
        found = False
    report(found, values)
    if found and values:
        return values
    return []
