def branch_raise(flag):
    if flag:
        try:
            action()
        except Exception:
            raise
    else:
        consume(flag)
    after(flag)

def branch_return(flag):
    if flag:
        try:
            action()
        except Exception:
            return
    else:
        consume(flag)
    after(flag)

def nested_loop(values):
    result = None
    for item in values:
        try:
            result = load(item)
        except KeyError:
            raise ValueError(item)
    return result
