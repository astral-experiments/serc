def f(futures, errors, outputs, progress):
    for future in futures:
        try:
            value = future.result()
        except Exception as exc:
            errors.append(exc)
        else:
            outputs.append(value)
        finally:
            progress.update(1)
