def target(subject):
    match subject:
        case {
            "first": str(first),
            "second": str(second),
            "third": str(third),
            "last": [str(last)],
        } if first and last:
            return first, second, third, last
    return None
