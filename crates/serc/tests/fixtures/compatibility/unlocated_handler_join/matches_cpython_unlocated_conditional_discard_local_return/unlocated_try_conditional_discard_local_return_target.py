def validate(value):
    try:
        decode(value) if value is not None else None
    except ValueError:
        return None
    return value
