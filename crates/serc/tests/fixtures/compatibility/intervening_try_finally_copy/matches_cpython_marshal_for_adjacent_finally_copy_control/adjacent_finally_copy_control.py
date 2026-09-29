async def fetch(session):
    try:
        response = await session.request()
        try:
            response.check()
            return await response.read()
        finally:
            response.release()
    finally:
        await session.close()
