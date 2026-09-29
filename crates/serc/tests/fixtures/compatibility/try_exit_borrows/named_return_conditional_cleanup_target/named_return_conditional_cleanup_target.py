async def probe(owner, token, result, touch):
    try:
        try:
            await touch(result)
        except BaseException:
            owner.release(result)
            raise
        return result
    finally:
        if token is not None:
            owner.remove(token)
