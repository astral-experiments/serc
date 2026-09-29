async def stream(manager, values, result, emit, cleanup, restore):
    try:
        with manager():
            try:
                async for value in values():
                    yield value
            except Exception:
                for value in await cleanup():
                    yield value
                raise
            finally:
                emit()
                if result is not None:
                    emit(result)
                    await cleanup()
    finally:
        restore()
