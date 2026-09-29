def f(value, result):
    match value:
        case "a":
            1; pass
        case "b":
            result.attr = 2
    return result
