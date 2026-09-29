def example(items, candidates, size, mode, obj, ready, alternate, use):
    if size > 0:
        before = prepare()
        if ready:
            use(before)
        middle = prepare()
        if alternate:
            use(middle)
        if mode == 0:
            total = 0
            picked = obj.value
            picked += total
            assert picked
        else:
            assert size
        result = finish(obj)
        use(result)
