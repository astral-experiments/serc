import contextlib

async def validate(items, open_item, work, mapping):
    value = None
    for item in items:
        app = None
        try:
            app = await open_item(item)
            value = await work(app)
            mapping["before"] = str(value)
            mapping["first"] = value
            mapping["second"] = value
            mapping["third"] = value
            break
        except Exception as error:
            value = str(error)
        finally:
            if app is not None:
                with contextlib.suppress(Exception):
                    await app.close()
    result = str(value)
    return result
