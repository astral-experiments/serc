def startup_first(value):
    if value in {None, "500", "ascii", "darwin", "kwargs", "l1", "l2", "update", "win32"}:
        return 1
    return 0


def startup_second(value):
    if value in {None, "500", "ascii", "darwin", "kwargs", "l1", "l2", "update", "win32"}:
        return 2
    return 0


def ordinary_first(value):
    if value in {None, "alpha", "elasticnet"}:
        return 3
    return 0


def ordinary_second(value):
    if value in {None, "alpha", "elasticnet"}:
        return 4
    return 0
