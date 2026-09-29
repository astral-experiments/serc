def conjunction(first, second, third):
    return first and (second and third)


def disjunction(first, second, third):
    return first or (second or third)


strip = lambda value: ((b'x' in value and value.strip()) or
                       ((b'x' not in value and value.lower()) or None))
