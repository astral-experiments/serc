async def f():
    with manager:
        try:
            async for output in values:
                pass
        except Error:
            pass
        finally:
            try:
                if flag:
                    pass
            finally:
                result = target.context.telemetry
                if result.value is None:
                    result.finalize(reason="done")
