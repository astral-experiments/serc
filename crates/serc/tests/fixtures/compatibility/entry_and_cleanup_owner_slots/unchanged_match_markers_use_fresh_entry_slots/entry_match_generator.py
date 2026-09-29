def f(subject):
    try:
        match subject:
            case "left":
                yield subject
            case _:
                yield None
    except Exception:
        yield None
