def f(subject):
    try:
        match subject:
            case "left":
                return lambda: subject
            case _:
                return None
    except Exception:
        return None
