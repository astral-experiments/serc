def f(items, verify, strict):
    for item in items:
        try:
            work(item)
            if verify:
                if strict:
                    assert check(item)
                else:
                    assert other(item)
        except ValueError as error:
            report(error)
