async def f(client, enabled, item, raise_on_error):
    try:
        await work(item)
    except Error as error:
        report(error)
        if raise_on_error:
            raise
        failed(item)
    finally:
        if enabled is not None and client is not None:
            try:
                await client.close()
            except Error:
                warning()
