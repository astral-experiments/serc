def f(items):
    for item in items:
        assert ("x" in item) or (item in values), "bad"
