async def iterator_loop_control(messages, service):
    for message in messages:
        try:
            parsed = validate(message)
        except ValueError:
            continue

        if (token := parsed.get("token")) and (
            session := service.validate(token)
        ):
            return session
    return None
