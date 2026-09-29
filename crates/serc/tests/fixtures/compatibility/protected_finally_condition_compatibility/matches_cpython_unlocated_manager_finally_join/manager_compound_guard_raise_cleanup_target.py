def store_checked(manager, operation):
    result = None
    with manager():
        try:
            result = operation()
        finally:
            if result is not None and not result.accepted:
                raise RuntimeError('rejected')
    return result
