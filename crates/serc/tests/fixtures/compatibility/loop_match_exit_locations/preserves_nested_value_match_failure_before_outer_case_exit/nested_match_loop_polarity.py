def f(tokens, low=3, high=7):
    count = 0
    for token in tokens:
        match token:
            case int():
                count += 1
            case dict():
                match token.get("type"):
                    case "low":
                        count += low
                    case "high":
                        count += high
    return count
