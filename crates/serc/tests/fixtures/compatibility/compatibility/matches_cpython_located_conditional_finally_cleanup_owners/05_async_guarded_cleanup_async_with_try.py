async def test(cm, timeout, client):
    with cm:
        try:
            await work()
        except Exception as exc:
            recover(exc)
        finally:
            if client is not None:
                try:
                    async with timeout:
                        await client.cleanup()
                except Exception as exc:
                    recover(exc)
