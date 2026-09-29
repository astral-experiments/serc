def f(subject, probe):
    try:
        match subject:
            case "a":
                return probe(subject)
            case None:
                return probe(subject)
    except TypeError:
        return False
    return False
