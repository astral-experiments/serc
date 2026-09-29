async def f(value, other, values, flag, manager, target):
    with manager:
        if True:
            consume(value)
        else:
            consume(other)
        try:
            async for item in values:
                consume(item)
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
