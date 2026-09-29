def protected_by_prior_call(groups):
    return [
        (probe(group), any(value for value in group))
        for group in groups
    ]
