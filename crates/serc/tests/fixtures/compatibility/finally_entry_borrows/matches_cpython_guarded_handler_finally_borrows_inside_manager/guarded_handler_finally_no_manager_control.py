def recover(items, progress, keep_going):
    results = {}
    for item in items:
        try:
            key, value = item.result()
            results[key] = value
        except Exception as error:
            results[item] = str(error)
            if not keep_going:
                raise
        finally:
            progress.update(1)
    return results
