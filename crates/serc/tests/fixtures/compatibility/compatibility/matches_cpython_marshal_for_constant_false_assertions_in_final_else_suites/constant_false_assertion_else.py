def direct(condition, value):
    if condition:
        return
    else:
        assert False, value


def chained(kind, value):
    if kind == "first":
        first()
    elif kind == "second":
        second()
    else:
        assert 0, f"unknown: {value}"


def control(value):
    assert False, value
