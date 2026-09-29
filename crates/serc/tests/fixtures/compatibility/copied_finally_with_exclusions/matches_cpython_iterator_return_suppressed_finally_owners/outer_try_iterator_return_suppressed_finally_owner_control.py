def run(values, worker, cleanup):
    try:
        for value in values:
            result = worker(value)
            try:
                return value, result
            finally:
                try:
                    cleanup.close()
                except OSError:
                    pass
    except LookupError:
        return None
    return None
