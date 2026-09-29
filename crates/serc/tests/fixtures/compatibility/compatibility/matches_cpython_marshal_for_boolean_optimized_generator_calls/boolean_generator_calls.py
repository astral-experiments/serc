def disjunction(prefix, values):
    return prefix or any(value for value in values)

def conjunction(prefix, values):
    return prefix and all(value for value in values)

def nested(first, second, values):
    return first or second or any(value for value in values)

wrapped = lambda first, second, values: first and second and all(value for value in values)
