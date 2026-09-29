async def fetch(session):
    try:
        response = await session.request()
        try:
            response.check()
            return await response.read()
        finally:
            response.release()
    except Exception as error:
        detail = str(error).strip()
        label = detail or type(error).__name__
        raise FetchError(f"Request failed: {label}") from error
    finally:
        await session.close()
