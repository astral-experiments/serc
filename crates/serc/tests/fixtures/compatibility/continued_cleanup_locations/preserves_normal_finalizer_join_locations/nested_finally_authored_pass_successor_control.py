async def nested_finalizer(items, selected, clean, notify):
    for item in items:
        try:
            work(item)
        finally:
            try:
                clean()
            finally:
                if selected(item):
                    await notify(item)
        pass
