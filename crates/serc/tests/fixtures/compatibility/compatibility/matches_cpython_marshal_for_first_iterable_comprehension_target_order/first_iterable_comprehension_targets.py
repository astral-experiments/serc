def first_iterable_targets(values):
    return [outer for outer in [inner for inner in values]]


def body_targets(values):
    return [[inner for inner in values] for outer in values]


def later_iterable_targets(values):
    return [(outer, inner) for outer in values for inner in [nested for nested in values]]


def generator_first_iterable(values):
    return [outer for outer in (inner for inner in values)]
