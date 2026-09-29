def select(subject, value, check):
    match subject:
        case 1:
            assert check(value)
            check(value.inner)
        case 2:
            assert check(value)
            check(value.inner)
        case _:
            raise AssertionError("unknown")
    return value.inner.item
