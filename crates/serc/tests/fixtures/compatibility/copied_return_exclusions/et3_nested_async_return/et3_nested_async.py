async def f(handle, enabled):
    try:
        try:
            description = await handle.describe()
            if description.status == enabled:
                await handle.terminate()
        except Exception:
            try:
                await handle.terminate()
            except Exception as err:
                report(err)
    except Exception as err:
        report(err)
