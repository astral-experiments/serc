def f(value, result, work):
    match value:
        case "a":
            work(); pass
        case "b":
            work()
    return result
