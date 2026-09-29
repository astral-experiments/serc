def convert_key(key, text_type, special_type, report):
    name: str | None
    if isinstance(key, text_type) and key.char is not None:
        name = key.char.upper()
    elif isinstance(key, special_type):
        name = key.name
    else:
        report(key)
        return None
    return name
