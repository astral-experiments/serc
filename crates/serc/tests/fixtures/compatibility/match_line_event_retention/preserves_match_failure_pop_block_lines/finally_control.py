def f(subject, probe):
    try:
        match subject:
            case "a":
                return probe(subject)
            case "b":
                return probe(subject)
    except TypeError:
        return False
    finally:
        probe(None)
    return False
