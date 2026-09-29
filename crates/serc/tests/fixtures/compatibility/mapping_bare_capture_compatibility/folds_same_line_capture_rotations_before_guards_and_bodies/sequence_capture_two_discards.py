def f(subject):
    match subject:
        case (captured, _, _):
            return captured
    return None
