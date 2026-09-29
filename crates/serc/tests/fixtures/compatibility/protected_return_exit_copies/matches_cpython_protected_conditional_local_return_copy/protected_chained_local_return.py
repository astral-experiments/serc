def select(value, upper):
    try:
        parsed = int(value)
        return parsed if 1 <= parsed <= upper else None
    except ValueError:
        return None
