def while_guard(values, ready):
    try:
        while ready:
            found = any(value for value in values)
            use(found)
    except Error:
        pass


def constant_while_guard(values):
    try:
        while True:
            found = all(value for value in values)
            use(found)
    except Error:
        pass


def prior_call_keeps_slot(values, ready):
    try:
        while ready:
            prepare()
            found = any(value for value in values)
            use(found)
    except Error:
        pass


def prior_unpack_keeps_slot(values, ready, pair):
    try:
        while ready:
            left, right = pair
            found = all(value for value in values)
            use(left, right, found)
    except Error:
        pass


def iterator_entry_keeps_slot(values):
    try:
        for entry in values:
            found = any(value for value in entry)
            use(found)
    except Error:
        pass
