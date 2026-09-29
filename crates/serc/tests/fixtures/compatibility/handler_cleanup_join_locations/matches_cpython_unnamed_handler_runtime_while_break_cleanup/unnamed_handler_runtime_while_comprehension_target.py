def run(load, recover, limit, stop):
    try:
        values = load()
    except RuntimeError:
        values = []
        offset = 0
        while offset < limit:
            value = recover(offset)
            values.append(value)
            offset += 1
            if stop(value):
                break
    return {item[0]: int(item[1]) for item in values if isinstance(item, tuple)}
