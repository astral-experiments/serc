def collect(load, walk, report):
    values = []
    try:
        root = load()
        if root:
            for value in walk(root):
                values.append(value)
        else:
            pass
    except Exception as error:
        report(error)
        return []

    return values
