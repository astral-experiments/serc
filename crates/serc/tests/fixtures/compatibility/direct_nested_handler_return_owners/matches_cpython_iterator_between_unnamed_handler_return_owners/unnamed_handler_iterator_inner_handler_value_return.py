def probe(items, operation, selected):
    try:
        operation()
    except ValueError:
        for item in items:
            try:
                operation(item)
            except TypeError:
                if selected(item):
                    return operation(item)
                continue
        return None
