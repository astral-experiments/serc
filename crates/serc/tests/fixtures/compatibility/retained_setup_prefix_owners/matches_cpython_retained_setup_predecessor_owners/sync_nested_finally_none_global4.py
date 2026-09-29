def f(resource, result, skip):
    try:
        if skip:
            return result
        tick_1()
        tick_2()
        tick_3()
        tick_4()
        return result
    finally:
        try:
            if resource is not None:
                resource.close()
        except Exception:
            pass
