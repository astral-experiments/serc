def run(items, work, recover):
    result = []
    for item in items:
        assert item is not None, 'stop'
        if item:
            try:
                result.append(work(item))
            except ValueError as error:
                recover(error)
        try:
            work(item)
        except TypeError:
            continue
    return result
