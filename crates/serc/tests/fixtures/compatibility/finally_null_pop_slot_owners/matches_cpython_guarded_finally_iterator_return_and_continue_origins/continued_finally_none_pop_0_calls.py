async def f(items, resource, recover):
    try:
        for item in items:
            try:
                if item:
                    continue
            finally:
                if resource is not None:
                    try:
                        await resource.close()
                    except Exception:
                        recover()
    except Exception:
        recover()
