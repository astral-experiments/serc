async def run(items, api):
    for item in items:
        try:
            if api.skip(item):
                api.record(item)
                continue
            api.use(item)
        finally:
            if item:
                api.close(item)
            api.finish_item()
