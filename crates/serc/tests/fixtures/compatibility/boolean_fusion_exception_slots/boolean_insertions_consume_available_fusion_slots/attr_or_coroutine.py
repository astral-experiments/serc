async def f(a, b, obj, auxiliary):
    sink(a, b)
    sink(a, b)
    sink(a, b)
    sink(a, b)
    try:
        value = obj.left or obj.right
        sink(value)
    except ValueError:
        recover()
    return obj
