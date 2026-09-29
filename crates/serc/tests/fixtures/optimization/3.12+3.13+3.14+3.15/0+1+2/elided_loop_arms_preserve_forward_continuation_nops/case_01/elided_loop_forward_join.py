def f(cm, items):
    for item in items:
        with cm:
            if item:
                assert item
            else:
                assert items
