async def run(resource):
    try:
        with manager():
            return build(value=resource)
    finally:
        if resource is not None:
            await resource.close()
