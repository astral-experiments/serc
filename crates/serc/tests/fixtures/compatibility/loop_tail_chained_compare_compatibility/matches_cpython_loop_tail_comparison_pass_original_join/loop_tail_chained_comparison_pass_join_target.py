def collect(items, limit, buckets):
    for index, value in enumerate(items):
        if 0 <= value < limit:
            buckets[value].append(index)
        else:
            pass
    finish(buckets)
