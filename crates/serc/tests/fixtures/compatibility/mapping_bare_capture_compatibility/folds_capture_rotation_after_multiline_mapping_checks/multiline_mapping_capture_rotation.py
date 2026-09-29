def f(subject):
    match subject:
        case {
            "kind": "target",
            "value": value,
        }:
            return value
        case _:
            return None
