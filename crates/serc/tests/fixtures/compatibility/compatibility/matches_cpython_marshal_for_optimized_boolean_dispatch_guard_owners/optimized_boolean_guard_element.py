def comprehension_element(groups):
    return [any(value for value in group) for group in groups]
