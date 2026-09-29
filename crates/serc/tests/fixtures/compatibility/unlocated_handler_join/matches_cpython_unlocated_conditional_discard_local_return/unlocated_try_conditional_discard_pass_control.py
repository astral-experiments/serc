def validate(value):
    try:
        decode(value) if value is not None else None
        pass
    except ValueError:
        return None
    return value
