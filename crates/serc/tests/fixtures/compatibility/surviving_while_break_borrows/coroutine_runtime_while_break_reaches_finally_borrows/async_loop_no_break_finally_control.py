async def lookup(client, sink):
    result = None
    try:
        while client.pending:
            result = await client.fetch()
            client.record(result)
    except Exception as error:
        client.failed(error)
    finally:
        sink.record(client)
    return result
