def f(value, ready, result, work):
    match value:
        case "a":
            if ready:
                work()
            pass
        case "b":
            work()
    return result
