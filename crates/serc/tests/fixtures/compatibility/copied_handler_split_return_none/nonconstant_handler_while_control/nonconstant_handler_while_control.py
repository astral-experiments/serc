async def run(manager, printer, observer, stop):
    async with manager:
        try:
            while True:
                try:
                    await action()
                except Error:
                    if stop is not None and stop():
                        printer.log("stopped")
                        return result
                    raise
        finally:
            if observer is not None:
                observer(None)
