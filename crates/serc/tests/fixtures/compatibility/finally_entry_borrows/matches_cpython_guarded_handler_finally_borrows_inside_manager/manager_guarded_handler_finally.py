def recover(items, manager, keep_going):
    results = {}
    with manager as progress:
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
