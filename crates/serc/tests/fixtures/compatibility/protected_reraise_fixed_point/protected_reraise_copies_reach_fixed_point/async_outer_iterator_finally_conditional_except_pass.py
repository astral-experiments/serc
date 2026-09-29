async def run(values, resource):
    for value in values:
        try:
            return await produce(value)
        except Exception as error:
            remember(error)
        finally:
            if resource is not None:
                closer = getattr(resource, "close", None)
                if closer is not None:
                    try:
                        result = closer()
                        if can_wait(result):
                            await result
                    except Exception:
                        pass
