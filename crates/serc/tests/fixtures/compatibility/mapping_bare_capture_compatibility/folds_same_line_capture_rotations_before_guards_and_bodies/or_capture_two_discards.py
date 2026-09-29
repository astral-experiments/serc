def f(subject):
    match subject:
        case ("red" | "blue") as captured, _, _:
            return captured
    return None
