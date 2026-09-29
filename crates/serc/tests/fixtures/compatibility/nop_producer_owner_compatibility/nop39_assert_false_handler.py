def f(items):
    for item in items:
        try:
            item()
        except Exception:
            assert (
                False
            ), "failed"
