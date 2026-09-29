def try_return():
    try:
        value = process()
        return value
    except ValueError:
        pass

def with_return(manager):
    with manager:
        return 1

def match_subject(provided):
    match provided:
        case True:
            return captured

def comprehension(collection):
    return [element for element in collection if element is not None]

def repeated_checked_load():
    value = value[value.attribute == 'key']

def maybe_unbound(condition):
    if condition:
        value = 1
    else:
        use()
    return value

def consecutive_try_imports():
    try:
        from first import First
    except ImportError:
        from fallback import First
    First()
    try:
        from second import Second
    except ImportError:
        from fallback import Second
    Second()

def folded_false(value):
    if value == 1:
        return 1
    elif False:
        return 2
    elif value == 3:
        return 3

def loaded_before_binding():
    for item in values:
        use(values, item)
    values = []

def conditional_import(flag):
    values = []
    if flag:
        values.append(flag)
    from package import imported
    return values, imported
