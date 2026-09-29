async def run(items, consume):
    for item in items:
        try:
            try:
                value = obtain(item)
            except Exception:
                value = None
            if isinstance(value, str) and value:
                await perform(value)
            consume(item)
        except Exception as error:
            consume(error)
