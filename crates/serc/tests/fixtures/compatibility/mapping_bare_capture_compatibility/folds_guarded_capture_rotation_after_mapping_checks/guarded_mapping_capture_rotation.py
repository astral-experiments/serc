def f(subject, accept):
    match subject:
        case {
            "kind": "target",
            "value": value,
        } if accept(value):
            return value
        case _:
            return None
