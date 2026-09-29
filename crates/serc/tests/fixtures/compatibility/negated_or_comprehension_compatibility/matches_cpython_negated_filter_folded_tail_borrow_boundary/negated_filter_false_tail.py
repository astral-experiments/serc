def collect(mapping, items):
    result = [item for item in items if not (mapping.get(item) or "")]
    return result
