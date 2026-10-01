def sample(seed, gate, value, items, others):
    if seed:
        assert value
    if gate:
        saved = value
        for item in items:
            saved = item
            for other in others:
                assert other
