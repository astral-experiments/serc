def rows(values):
    for value in values:
        value["flag"] = not (
            total(value) >= 2
        )
        yield value
