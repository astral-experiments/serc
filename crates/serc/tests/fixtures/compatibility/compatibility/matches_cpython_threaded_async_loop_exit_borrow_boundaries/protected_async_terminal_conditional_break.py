async def run(manager, trace, source, value):
    async with manager:
        with trace:
            matched = False
            try:
                async for item in source:
                    if selected(item) and item.key == value:
                        matched = True
                        break
            except (Error, OtherError):
                pass
            if not matched:
                consume(value)
            consume(source)
