def f(resource, pairs, sink):
    try:
        mapped = {str(key): int(value) for key, value in pairs}
        result = third(second(first(mapped)))
    finally:
        if resource is not None:
            resource.close()
    sink(result)
