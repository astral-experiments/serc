async def f(items, ready, value, resource):
    for item in items:
        try:
            if value:
                raise ValueError
            result = make_result(value=value)
            ready = not value
            return result
        except Exception:
            raise
        finally:
            if resource is not None:
                await cleanup(resource)
