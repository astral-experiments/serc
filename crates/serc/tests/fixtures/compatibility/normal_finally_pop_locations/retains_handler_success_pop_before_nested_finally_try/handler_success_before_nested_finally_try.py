async def target(make, record, attempts):
    for attempt in attempts:
        client = None
        try:
            client = make()
            await client.read()
            record(None)
            return
        except Exception as error:
            retry = bool(attempt)
            record(
                error,
                retry,
            )
        finally:
            try:
                if client is not None:
                    await client.close()
            except Exception as error:
                record(error)
