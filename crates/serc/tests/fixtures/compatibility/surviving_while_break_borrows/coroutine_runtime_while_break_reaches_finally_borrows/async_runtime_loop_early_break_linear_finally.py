async def lookup(client, sink):
    result = None
    try:
        cursor = None
        while result is None:
            result, next_cursor = await client.fetch(cursor)
            if result is not None or next_cursor is None:
                break
            client.record(next_cursor)
            cursor = next_cursor
    except Exception as error:
        client.failed(error)
        raise
    finally:
        sink.record(client)
    return result
