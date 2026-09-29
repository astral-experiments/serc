async def f(manager, values, request, decode, pause):
    async with manager:
        for value in values:
            payload = await request(value)
            try:
                result = decode(payload)
                return {"result": result}
            except ValueError as error:
                if value:
                    return {"error": str(error)}
                continue
    return {}
