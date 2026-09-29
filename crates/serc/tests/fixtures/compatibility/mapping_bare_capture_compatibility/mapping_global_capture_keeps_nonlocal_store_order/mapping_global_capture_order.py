def f(subject):
    global value
    match subject:
        case {
            "kind": "target",
            "value": value,
        }:
            return value
        case _:
            return None
