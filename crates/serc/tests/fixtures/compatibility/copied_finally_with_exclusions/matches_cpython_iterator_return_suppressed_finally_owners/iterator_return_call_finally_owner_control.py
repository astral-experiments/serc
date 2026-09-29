def run(values, worker, cleanup):
    for value in values:
        try:
            result = worker(value)
            try:
                return value, result
            finally:
                cleanup.close()
        except LookupError:
            continue
    return None
