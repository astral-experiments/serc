async def run(items, resource):
    for item in items:
        try:
            value = await obtain(item)
            return finish(value).result
        except Error as error:
            if retry(error):
                await pause()
        finally:
            if resource is not None:
                close = getattr(resource, "close", None)
                if close is not None:
                    await close()
