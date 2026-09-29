def collect(value, manager, items, flag):
    try:
        try:
            try:
                prepare(value)
            except Error:
                return value
        finally:
            pass
    finally:
        with manager:
            pass
