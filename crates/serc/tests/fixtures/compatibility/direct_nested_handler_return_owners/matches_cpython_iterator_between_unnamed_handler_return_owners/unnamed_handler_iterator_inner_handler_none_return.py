def probe(items, operation):
    try:
        operation()
    except ValueError:
        for item in items:
            try:
                operation(item)
            except TypeError:
                return None
        return None
