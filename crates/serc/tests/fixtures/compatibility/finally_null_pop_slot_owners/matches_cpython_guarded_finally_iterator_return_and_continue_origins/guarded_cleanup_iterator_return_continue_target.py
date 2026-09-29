async def f(items, first, second):
    try:
        for item in items:
            try:
                if item:
                    return item
                continue
            finally:
                if first is not None:
                    try:
                        await first.close()
                    except Exception as error:
                        report(error)
                if second is not None:
                    try:
                        await second.close()
                    except Exception as error:
                        report(error)
    except Exception:
        raise
