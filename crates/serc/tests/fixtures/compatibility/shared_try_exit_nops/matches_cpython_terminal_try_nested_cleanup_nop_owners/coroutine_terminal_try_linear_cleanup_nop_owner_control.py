async def run(client, value):
    try:
        await client.prepare()
        if not isinstance(value, str) or not value:
            raise ValueError(value)
    except Exception as error:
        raise RuntimeError("failed") from error
    finally:
        await client.close()
