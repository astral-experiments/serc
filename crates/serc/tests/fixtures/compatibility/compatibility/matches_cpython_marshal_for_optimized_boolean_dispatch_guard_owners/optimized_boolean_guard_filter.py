def simple_filter(groups):
    return [group for group in groups if any(value for value in group)]
