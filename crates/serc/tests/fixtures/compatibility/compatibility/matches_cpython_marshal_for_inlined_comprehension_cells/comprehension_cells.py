module_result = [lambda: x for x in range(2)]

def captured_targets():
    lists = [lambda: x for x in range(2)]
    sets = {lambda: x for x in range(2)}
    dictionaries = {x: lambda: x for x in range(2)}
    return lists, sets, dictionaries

def unreachable_target():
    if False:
        return [lambda: item for item in range(3)]

def nested_targets(xs, ys):
    return [[lambda: x for x in ys] for x in xs]

def target_rebound_as_later_cell(types, values):
    normalized = {normalize(type_) for type_ in types}
    for type_ in types:
        converted = (convert(value, type_) for value in values)
    return normalized, converted
