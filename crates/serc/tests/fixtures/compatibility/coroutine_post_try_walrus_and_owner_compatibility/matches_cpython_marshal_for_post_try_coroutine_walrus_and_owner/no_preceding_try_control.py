async def no_preceding_try_control(message, service):
    parsed = validate(message)
    if (token := parsed.get("token")) and (
        session := service.validate(token)
    ):
        return session
    return None
