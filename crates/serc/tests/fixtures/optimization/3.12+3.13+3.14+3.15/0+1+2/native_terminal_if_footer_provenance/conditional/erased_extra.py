def sample(seed, gate, value, items, others):
    if seed:
        assert value
    if gate:
        if value:
            saved = value
        for item in items:
            assert item
            for other in others:
                assert other
