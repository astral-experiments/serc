def f(resource, a, b, x, y):
    def capture():
        return x, y
    try:
        value = work(a, b)
        value = work(a, b)
        value = work(a, b)
        return value
    finally:
        if resource is not None:
            resource.close()
        cleanup()
