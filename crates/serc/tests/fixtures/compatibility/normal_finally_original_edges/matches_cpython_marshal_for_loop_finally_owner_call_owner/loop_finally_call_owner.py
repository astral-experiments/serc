async def f(manager, values, writer, kind):
    with manager:
        try:
            for value in values:
                await consume(value)
                if value.stop:
                    break
        finally:
            if isinstance(writer, kind):
                writer.close()
            finish()
