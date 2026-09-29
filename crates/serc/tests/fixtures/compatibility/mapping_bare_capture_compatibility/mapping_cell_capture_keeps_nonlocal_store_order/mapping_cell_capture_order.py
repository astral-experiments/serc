def f(subject):
    def capture():
        return value
    match subject:
        case {
            "kind": "target",
            "value": value,
        }:
            return capture()
        case _:
            return None
