async def f(outer, inner, values, work, retry, report):
    try:
        async with outer():
            for item in values:
                try:
                    async with inner(item):
                        try:
                            await work(item)
                        except ValueError as error:
                            if retry(error):
                                continue
                            report(error)
                            return None
                except RuntimeError:
                    report(item)
    except LookupError:
        report(None)
