async def stream(values, manager, active, asynchronous):
    try:
        yield await next_item()
    finally:
        for closer in values:
            if active:
                with manager:
                    if asynchronous:
                        await closer()
                    else:
                        closer()
                break
