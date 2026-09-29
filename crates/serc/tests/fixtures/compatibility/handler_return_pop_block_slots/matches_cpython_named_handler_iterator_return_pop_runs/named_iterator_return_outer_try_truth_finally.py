async def f(items, result, resource):
    try:
        for item in items:
            try:
                await work(item)
            except Exception as error:
                if item:
                    result["error"] = f"failed: {error}"
                    return 1, result
    except Exception as error:
        return 2, result
    finally:
        if resource:
            cleanup(resource)
