def target(subject):
    match subject:
        case {"left": {"value": left}, "right": {"value": right}}:
            return left, right
    return None


def different_cleanup_lines(left, right):
    match left, right:
        case (
            {"value": str(first)},
            {
                "value": str(second),
                "extra": str(third),
            },
        ):
            return first, second, third
    return None
