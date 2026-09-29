def select(value, upper):
    try:
        parsed = int(value)
        return parsed if parsed <= upper else None
    except ValueError:
        return None
