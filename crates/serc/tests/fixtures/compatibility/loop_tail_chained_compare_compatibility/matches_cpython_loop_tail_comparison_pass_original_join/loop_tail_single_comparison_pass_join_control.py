def collect(items, limit, buckets):
    for index, value in enumerate(items):
        if value < limit:
            buckets[value].append(index)
        else:
            pass
    finish(buckets)
