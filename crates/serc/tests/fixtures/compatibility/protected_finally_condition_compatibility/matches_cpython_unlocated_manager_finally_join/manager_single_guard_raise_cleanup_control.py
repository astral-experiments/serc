def store_checked(manager, operation, failed):
    result = None
    with manager():
        try:
            result = operation()
        finally:
            if failed:
                raise RuntimeError('rejected')
    return result
