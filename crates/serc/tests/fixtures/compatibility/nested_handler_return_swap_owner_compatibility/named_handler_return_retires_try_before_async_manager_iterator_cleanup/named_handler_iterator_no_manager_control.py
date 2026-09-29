async def f(manager, values, request, decode, pause):
    for value in values:
        try:
            payload = await request(value)
            try:
                result = decode(payload)
                return {"result": result}
            except ValueError as error:
                if value:
                    return {"error": str(error)}
                continue
        except Exception as error:
            if value:
                return {"error": str(error)}
            await pause()
    return {}
