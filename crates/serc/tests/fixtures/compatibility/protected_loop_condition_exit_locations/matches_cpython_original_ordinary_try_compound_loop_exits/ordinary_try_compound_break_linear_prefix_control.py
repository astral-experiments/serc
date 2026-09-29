def run(items, limit, recover):
    for item in items:
        try:
            recover(item)
            if limit is not None and item >= limit:
                break
        except Exception:
            continue
    return None
