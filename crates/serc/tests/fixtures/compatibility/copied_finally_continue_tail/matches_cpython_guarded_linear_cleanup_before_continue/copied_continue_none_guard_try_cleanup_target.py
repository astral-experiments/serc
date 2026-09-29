async def f(items, client):
    for item in items:
        try:
            if item:
                continue
            return item
        finally:
            if client is not None:
                try:
                    await client.close()
                except Exception as error:
                    report(error)
