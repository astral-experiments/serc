def f(value):
    match value:
        case C(
            values=[C(items=[_, a]), b]):
            return a, b
