def f(cm, items):
    with cm:
        for item in items:
            if item:
                assert item
            else:
                assert items
            work(item)
