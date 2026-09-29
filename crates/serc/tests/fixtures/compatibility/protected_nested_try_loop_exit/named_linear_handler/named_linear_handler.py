def f(items, outer, inner, consume, recover):
    for item in items:
        try:
            if outer(item):
                if inner(item):
                    consume(item)
        except Exception as exc:
            recover(exc)
            failures.append(item)
