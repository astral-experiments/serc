def filtered(values, predicate, selector):
    return [value for value in values if (predicate(value) if selector else True)]
