async def parse_context(session):
    value = Model.validate(session.primary or session.secondary or {})
    return value
