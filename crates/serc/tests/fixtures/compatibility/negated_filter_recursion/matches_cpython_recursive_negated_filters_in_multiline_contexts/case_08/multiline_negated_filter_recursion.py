class Sample:
    result = {
        x: transform(x)
        for x in items
        if (
            not (first(x) or (second(x) and third(x)))
        )
    }
