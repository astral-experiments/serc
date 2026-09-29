def f(value):
    match value:
        case C(
            values=[C(items=[_, a]), b]) if value:
            return a, b
