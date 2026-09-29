async def f(manager):
    try:
        client = create()
        context = client.context()
        await work()
    except Exception:
        pass
    finally:
        with manager:
            cleanup(client, context)
