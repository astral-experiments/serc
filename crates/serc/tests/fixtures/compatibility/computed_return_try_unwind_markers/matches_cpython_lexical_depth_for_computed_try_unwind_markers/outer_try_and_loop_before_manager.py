def f(managers, value, ready, fallback):
    for manager in managers:
        try:
            with manager:
                try:
                    return value if ready else fallback
                except Error:
                    return None
        except Outer:
            raise
