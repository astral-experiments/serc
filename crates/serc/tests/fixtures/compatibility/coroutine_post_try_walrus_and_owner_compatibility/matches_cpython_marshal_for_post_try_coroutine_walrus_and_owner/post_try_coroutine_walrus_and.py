async def post_try_coroutine_walrus_and(message, service):
    try:
        parsed = validate(message)
    except ValueError as error:
        await report(error)
        raise

    if (token := parsed.get("token")) and (
        session := service.validate(token)
    ):
        return session
    return None
