def make_query(client):
    async def query(limit=None, filters=None, order='DESC'):
        result = await client.fetch(limit=limit, filters=filters, order=order)
        assert limit is None or len(result) <= limit
        return result
    return query
