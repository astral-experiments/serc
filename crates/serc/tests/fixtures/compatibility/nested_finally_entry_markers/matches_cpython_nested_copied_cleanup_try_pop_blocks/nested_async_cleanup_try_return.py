async def probe(manager, inner, outer, value):
    async with manager():
        try:
            try:
                return value
            finally:
                try:
                    await inner.aclose()
                except Exception:
                    pass
        finally:
            try:
                outer.close()
            except Exception:
                pass
