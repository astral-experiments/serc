async def collect(items):
    try:
        try:
            await prepare()
            return build(), describe()
        finally:
            errors = []
            for item in items:
                try:
                    await item.close()
                except Exception as error:
                    errors.append(error)
            if errors:
                raise ExceptionGroup("cleanup failed", errors)
    finally:
        finish()
