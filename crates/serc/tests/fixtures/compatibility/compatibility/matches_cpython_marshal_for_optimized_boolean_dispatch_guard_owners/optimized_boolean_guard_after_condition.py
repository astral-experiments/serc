def after_condition(groups):
    return [
        group
        for group in groups
        if probe(group)
        if any(value for value in group)
    ]
