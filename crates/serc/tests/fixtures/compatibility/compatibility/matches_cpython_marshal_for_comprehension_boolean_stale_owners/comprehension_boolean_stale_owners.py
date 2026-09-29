def unowned(items):
    return [int(item or 0) for item in items]

def owned(items):
    return [(int(timestamp), int(count or 0)) for count, timestamp in items]
