def make_query(client):
    async def query(limit=None, filters=None, order='DESC'):
        result = await client.fetch(limit=limit, filters=filters, order=order)
        assert len(result) <= limit if limit is not None else True
        return result
    return query
