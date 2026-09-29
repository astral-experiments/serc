async def run(client, code):
    try:
        await client.prepare()
        if code != 0:
            client.warn(code)
            yield "failed"
            return
    except Exception as error:
        yield str(error)
        return
    finally:
        try:
            await client.close()
        except Exception:
            client.warn("cleanup")
    yield "finished"
