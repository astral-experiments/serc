def rows(values):
    for value in values:
        value["flag"] = not (
            total(value)
            >= limit(value)
        )
        yield value
