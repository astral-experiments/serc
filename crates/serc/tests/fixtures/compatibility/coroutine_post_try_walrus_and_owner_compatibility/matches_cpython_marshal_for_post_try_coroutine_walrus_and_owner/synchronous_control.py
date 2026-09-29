def synchronous_control(message, service):
    try:
        parsed = validate(message)
    except ValueError:
        raise

    if (token := parsed.get("token")) and (
        session := service.validate(token)
    ):
        return session
    return None
