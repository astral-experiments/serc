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
            assert total
            picked = 0
            for candidate in candidates:
                assert candidate
                if candidate == mode:
                    picked = candidate.value
                    picked += total
                    if picked == mode:
                        break
            assert picked
        else:
            assert size
        result = finish(obj)
        use(result)
