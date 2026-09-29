def f(subject, probe):
    match subject:
        case _ if (
            found := probe(subject)
        ) is not None:
            return found
        case _:
            return 0
