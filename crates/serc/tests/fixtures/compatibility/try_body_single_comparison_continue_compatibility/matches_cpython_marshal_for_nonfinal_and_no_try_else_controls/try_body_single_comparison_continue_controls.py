def nonfinal_control(items, mapping):
    for item in items:
        try:
            if item in mapping:
                continue
            prepare(item)
        except Error:
            recover(item)
        else:
            process(item)

def no_try_else_control(items, mapping):
    for item in items:
        try:
            if item in mapping:
                continue
        except Error:
            recover(item)
