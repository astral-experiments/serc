def run(manager, items, ready, valid, fallback):
    with manager:
        for item in items:
            assert valid(item) if ready(item) else fallback(item)
