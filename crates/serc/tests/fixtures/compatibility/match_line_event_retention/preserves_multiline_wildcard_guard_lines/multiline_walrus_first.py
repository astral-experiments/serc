def f(subject, probe):
    match subject:
        case _ if (
            found := probe(subject)
        ) is not None:
            return found
        case 1:
            return 1
        case _:
            return 0
