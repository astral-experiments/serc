def f(manager, value):
    with manager:
        try:
            try:
                return value
            except Inner:
                raise
        except Outer:
            return None
