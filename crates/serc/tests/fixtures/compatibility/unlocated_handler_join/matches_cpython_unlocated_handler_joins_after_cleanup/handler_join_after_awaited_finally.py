async def run(items, manager, send, close, finish):
    for item in items:
        try:
            await send(item)
        except Exception:
            try:
                with manager:
                    await close(item)
            finally:
                finish(item)
