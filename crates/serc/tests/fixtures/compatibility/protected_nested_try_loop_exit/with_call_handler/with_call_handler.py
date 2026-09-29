def f(items, outer, inner, consume, recover, manager):
    with manager:
        for item in items:
            try:
                if outer(item):
                    if inner(item):
                        consume(item)
            except Exception:
                recover(item)
