match shopper:
    case "Jane":
        consume(shopper)
        if condition:
            consume(condition)
    case _:
        fallback(shopper)
