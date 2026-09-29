def f(manager, subject, consume):
    with manager:
        match subject:
            case (captured, _, _):
                if captured is None:
                    consume(captured)
            case _:
                consume(None)
