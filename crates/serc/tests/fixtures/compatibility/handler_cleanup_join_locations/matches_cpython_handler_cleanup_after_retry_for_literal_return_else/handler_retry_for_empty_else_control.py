def parse_value(value, formats, primary, fallback):
    try:
        parsed = primary(value)
    except ValueError:
        for fmt in formats:
            try:
                parsed = fallback(value, fmt)
                break
            except ValueError:
                continue
    if parsed.ready is None:
        parsed = parsed.replace()
    return parsed.finish()
