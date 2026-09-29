def f(subject, convert):
    match subject:
        case {"outer": {"inner": captured}}:
            convert(captured)
            try:
                result = convert(captured)
            except ValueError:
                result = captured
            return result
    return None
