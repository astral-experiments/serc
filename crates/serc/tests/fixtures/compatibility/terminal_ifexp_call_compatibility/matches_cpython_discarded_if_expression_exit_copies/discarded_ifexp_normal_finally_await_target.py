async def target(resource):
    try:
        await work()
    finally:
        await resource.close() if resource else None
    return build_result()
