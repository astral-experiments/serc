async def build(items, cleanup):
    try:
        return {name: count for name, count in items if count}
    finally:
        cleanup()
