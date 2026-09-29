def filtered(values, predicate, selector):
    return [value for value in values if (not predicate(value) if selector else True)]
