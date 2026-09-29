async def worker(manager, ready, read, report, client):
    skip = False
    with manager:
        try:
            while ready():
                try:
                    read()
                except Exception as error:
                    skip = True
                    report(error)
                    return
        except OSError:
            report(None)
        finally:
            if skip:
                return
            report(None)
            await client.finish()
