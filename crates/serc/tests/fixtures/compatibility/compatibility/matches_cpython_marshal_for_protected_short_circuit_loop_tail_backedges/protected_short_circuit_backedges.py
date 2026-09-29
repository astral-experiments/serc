def collect(items):
    output = {}
    for item in items:
        try:
            name = item.name
            version = item.version
            if name and version:
                output[name] = version
        except Exception:
            recover(item)
    return output
