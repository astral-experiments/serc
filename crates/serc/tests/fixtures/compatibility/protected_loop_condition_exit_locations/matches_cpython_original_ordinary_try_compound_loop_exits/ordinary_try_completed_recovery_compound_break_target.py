def run(items, limit, recover):
    for item in items:
        try:
            try:
                recover(item)
            except LookupError:
                pass
            if limit is not None and item >= limit:
                break
        except Exception:
            continue
    return None
