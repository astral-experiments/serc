async def f(futures, errors, outputs, progress):
    try:
        for future in futures:
            try:
                value = await future
            except Exception as exc:
                errors.append(exc)
            else:
                if value is None:
                    errors.append(value)
                else:
                    outputs.append(value)
                    if value.ready():
                        flush()
            finally:
                progress.update(1)
    finally:
        progress.close()
