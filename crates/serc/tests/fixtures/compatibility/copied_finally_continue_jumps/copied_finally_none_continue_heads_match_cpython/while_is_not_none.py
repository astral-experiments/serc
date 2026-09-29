async def consume(items, active, resource, work, close, notify, done):
    while active:
        try:
            await work(resource)
        except Exception as exc:
            notify(exc)
            continue
        finally:
            if resource is not None:
                try:
                    await close(resource)
                except Exception as error:
                    notify(error)
    return resource
