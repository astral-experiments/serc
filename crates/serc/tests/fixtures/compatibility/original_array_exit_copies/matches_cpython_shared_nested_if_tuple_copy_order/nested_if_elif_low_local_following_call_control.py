def f(outer, choose, sort, model, left, right):
    if outer:
        if choose or model:
            left = prepare(left)
            if sort is not None:
                left.apply(lambda value: restore(value, sort))
            elif getattr(model, "output", None) is not None:
                left.apply(lambda value: trim(value, model))
        else:
            left = empty()
    else:
        left = fallback(right)
    observe(left)
    return left, right
