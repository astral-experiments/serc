def f(resource, a, b):
    scale = (1 << 64) - 1
    try:
        value = work(a, b)
        value = work(a, b)
    finally:
        if resource is not None:
            resource.close()
        cleanup()
    return scale
