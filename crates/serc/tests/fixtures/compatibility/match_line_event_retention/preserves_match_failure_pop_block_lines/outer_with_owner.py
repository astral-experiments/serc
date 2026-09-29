def f(subject, probe, manager):
    with manager:
        try:
            match subject:
                case "a":
                    return probe(subject)
                case "b":
                    return probe(subject)
        except TypeError:
            return False
        return False
