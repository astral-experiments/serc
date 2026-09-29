def f(subject, probe):
    try:
        match subject:
            case _ if (
                found := probe(subject)
            ) is not None:
                return found
            case 1:
                return 1
            case _:
                return 0
    except Exception:
        return -1
