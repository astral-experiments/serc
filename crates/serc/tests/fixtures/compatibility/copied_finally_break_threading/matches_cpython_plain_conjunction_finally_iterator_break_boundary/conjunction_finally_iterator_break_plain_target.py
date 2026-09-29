def collect(items, ready, failed, callback):
    for item in items:
        try:
            if item:
                break
        finally:
            if ready and not failed and callback is not None:
                callback(item, ready)
    return items
