async def run(items, make, retry):
    for item in items:
        handle = None
        try:
            handle = await make(item)
            yield handle
            return
        except Exception as error:
            if retry(error):
                continue
            break
        finally:
            if handle is not None:
                await close_handle(handle)
    raise RuntimeError
