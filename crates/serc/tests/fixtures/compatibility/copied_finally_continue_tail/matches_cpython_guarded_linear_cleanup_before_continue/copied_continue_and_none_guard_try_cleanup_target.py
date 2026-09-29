async def f(items, token, client):
    for item in items:
        try:
            if item:
                continue
            work(item)
        except Exception as error:
            report(error)
        finally:
            cleanup_client = client
            if token is not None and cleanup_client is not None:
                try:
                    await cleanup_client.close(token)
                    mark_done()
                except Exception as error:
                    report(error)
