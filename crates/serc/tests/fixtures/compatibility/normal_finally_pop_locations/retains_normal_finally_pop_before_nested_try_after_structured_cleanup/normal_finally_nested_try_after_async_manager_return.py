async def target(manager, work, finish):
    try:
        async with manager:
            value = await work()
            return value
    except TimeoutError:
        return None
    except RuntimeError:
        raise
    except Exception:
        return None
    finally:
        try:
            finish()
        except Exception:
            pass
