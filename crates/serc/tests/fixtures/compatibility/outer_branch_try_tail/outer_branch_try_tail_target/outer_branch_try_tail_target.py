def normalize(value):
    if isinstance(value, str):
        text = value.strip()
        if text.lower() in {"", "none"}:
            return None
        try:
            if int(text) < 0:
                return None
        except ValueError:
            pass
    elif type(value) is int and value < 0:
        return None
    return value
