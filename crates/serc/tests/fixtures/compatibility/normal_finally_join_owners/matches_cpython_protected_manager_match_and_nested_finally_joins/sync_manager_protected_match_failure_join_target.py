async def protected_match(manager, value):
    with manager:
        try:
            match value:
                case First():
                    await handle_first(value)
                case Second():
                    await handle_second(value)
        except Exception as error:
            record(error)
            raise
    finish()
