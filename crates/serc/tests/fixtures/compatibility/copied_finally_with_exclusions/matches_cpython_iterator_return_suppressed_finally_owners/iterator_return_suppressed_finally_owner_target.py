def run(values, worker, cleanup):
    for value in values:
        try:
            result = worker(value)
            try:
                return value, result
            finally:
                try:
                    cleanup.close()
                except OSError:
                    pass
        except LookupError:
            continue
    return None
