async def lookup(client, sink):
    result = None
    try:
        cursor = None
        while result is None:
            batch = await client.fetch(cursor)
            result = next((item for item in batch.items if item.ready), None)
            next_cursor = batch.cursor
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
