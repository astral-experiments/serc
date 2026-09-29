def f(manager, subject, accept, consume):
    with manager:
        match subject:
            case {
                "kind": "target",
                "value": value,
            }:
                if accept(value):
                    consume(value)
            case _:
                consume(None)
