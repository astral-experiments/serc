def f(subject, consume):
    match subject:
        case {
            "kind": "first",
            "value": value,
        }:
            consume(value)
        case {
            "kind": "second",
            "value": value,
        }:
            consume(value)
        case _:
            return None
    return value
