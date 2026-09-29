def target(items, ready, limit, consume):
    for item in items:
        try:
            if ready(item) and 0 <= item < limit:
                consume(item)
        except Exception:
            pass


def existing_flat_boolean(items, ready, limit, consume):
    for item in items:
        try:
            if ready(item) and item < limit:
                consume(item)
        except Exception:
            pass
