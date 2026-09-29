def f(outer, inner, item):
    try:
        with outer:
            with inner:
                try:
                    work()
                except ValueError:
                    report(item)
                    return 0
    finally:
        if item is not None:
            item.close()
