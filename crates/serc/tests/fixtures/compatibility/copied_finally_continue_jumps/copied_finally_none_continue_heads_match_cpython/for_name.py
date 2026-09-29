async def consume(items, active, resource, work, close, notify, done):
    for unused in items:
        try:
            await work(resource)
        except Exception as exc:
            notify(exc)
            continue
        finally:
            if resource:
                try:
                    await close(resource)
                except Exception as error:
                    notify(error)
    return resource
