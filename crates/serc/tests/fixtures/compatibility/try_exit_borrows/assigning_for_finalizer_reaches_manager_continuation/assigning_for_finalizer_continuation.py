def f(manager, items, total, work, observe):
    with manager:
        try:
            work(total)
        finally:
            for item in items:
                total += item
                observe(total)
    observe(total)
    observe(total)
    return total
