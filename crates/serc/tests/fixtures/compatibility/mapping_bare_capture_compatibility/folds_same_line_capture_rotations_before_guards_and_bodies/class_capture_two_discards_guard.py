def f(subject):
    match subject:
        case list() as captured, _, _ if captured:
            return captured
    return None
