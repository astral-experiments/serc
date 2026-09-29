def generator_disjunction(items, left, right):
    return (item for item in items if item == left or item == right)

def generator_chained(items, lower, upper, excluded):
    return (
        item
        for item in items
        if lower < item < upper and item != excluded
    )

def generator_ranges(items, first, lower, upper, last):
    return (
        item
        for item in items
        if item == first or lower <= item <= upper or item == last
    )

def negated_inlined(items, marker, other):
    return [item for item in items if not (item is marker and item == other)]

def negated_generator(items, marker, other):
    return (item for item in items if not (item is marker and item == other))

