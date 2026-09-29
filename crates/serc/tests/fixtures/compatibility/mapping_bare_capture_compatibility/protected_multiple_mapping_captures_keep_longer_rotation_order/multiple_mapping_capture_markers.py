def f(manager, subject, accept, consume):
    with manager:
        match subject:
            case {
                "kind": "target",
                "first": first,
                "last": last,
            }:
                if accept(first):
                    consume(first, last)
            case _:
                consume(None, None)
