def f(handle, enabled):
    try:
        try:
            description = handle.describe()
            if description.status == enabled:
                handle.terminate()
        except Exception:
            try:
                handle.terminate()
            except Exception as err:
                report(err)
    except Exception as err:
        report(err)
