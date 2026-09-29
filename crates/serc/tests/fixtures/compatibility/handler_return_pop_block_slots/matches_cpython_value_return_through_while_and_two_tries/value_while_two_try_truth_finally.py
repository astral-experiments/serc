async def f(ready, value, resource):
    try:
        while ready:
            try:
                try:
                    return make_result(value=convert(value), success=True)
                except ValueError:
                    raise
            except Exception:
                raise
    finally:
        if resource:
            cleanup(resource)
