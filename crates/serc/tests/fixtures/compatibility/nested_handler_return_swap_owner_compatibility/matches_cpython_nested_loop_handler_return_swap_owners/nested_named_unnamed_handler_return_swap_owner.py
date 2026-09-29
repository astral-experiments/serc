async def example(values):
    for value in values:
        try:
            await prepare(value)
            try:
                return primary(value)
            except Exception as error:
                try:
                    return secondary(value)
                except Exception:
                    if value:
                        try:
                            return tertiary(value, error)
                        except Exception:
                            pass
                    continue
        except Exception:
            continue
