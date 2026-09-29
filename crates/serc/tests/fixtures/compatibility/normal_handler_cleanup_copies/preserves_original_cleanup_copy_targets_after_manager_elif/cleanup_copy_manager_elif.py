def probe(manager, value, kind, action, report):
    try:
        if value is not None:
            if manager is None:
                return "no manager"
            with manager:
                state = action()
                if state == 1:
                    return "one"
                if state == 2:
                    return "two"
                action()
        elif kind == 1:
            action()
        else:
            report()
    except LookupError:
        return "missing"
    except OSError as error:
        return report(error)
    return "done"
