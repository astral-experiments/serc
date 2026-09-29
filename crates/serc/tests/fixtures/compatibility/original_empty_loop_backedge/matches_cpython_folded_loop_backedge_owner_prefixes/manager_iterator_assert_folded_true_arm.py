def run(manager, items, ready, valid):
    with manager:
        for item in items:
            assert valid(item) if ready(item) else 1.0
