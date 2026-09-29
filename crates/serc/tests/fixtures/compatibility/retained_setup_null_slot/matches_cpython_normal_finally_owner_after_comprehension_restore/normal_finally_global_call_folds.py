def f(resource, value):
    try:
        result = third(second(first(value)))
    finally:
        if resource is not None:
            resource.close()
    consume(result)
