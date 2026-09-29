async def f(manager):
    with manager:
        try:
            prepare()
            try:
                result = await fetch()
                return (
                    accept()
                    if result
                    else reject()
                )
            except ValueError:
                recover()
            return fallback()
        except TypeError:
            return fallback()
