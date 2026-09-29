async def run(manager, printer, observer, stop):
    async with manager:
        try:
            try:
                await action()
            except Error:
                if stop is not None and stop():
                    printer.log("stopped")
                    return 130
                raise
        finally:
            if observer is not None:
                observer(None)
