def f(a, b, obj, auxiliary):
    def capture():
        return auxiliary
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
