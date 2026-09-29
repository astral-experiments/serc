def f(a, b, obj, auxiliary):
    try:
        sink(a, b)
        sink(a, b)
        sink(a, b)
        sink(a, b)
        try:
            value = obj.left or obj.right
            sink(value)
        except ValueError:
            recover()
    except TypeError:
        outer_recover()
    return obj
