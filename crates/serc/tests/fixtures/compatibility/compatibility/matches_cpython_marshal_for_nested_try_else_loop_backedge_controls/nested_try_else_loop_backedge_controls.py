def final_conditional_else(values):
    for value in values:
        try:
            callback = lookup(value)
        except KeyError:
            pass
        else:
            prepare(callback)
            if callback(value):
                consume(value)
            else:
                reject(value)


def constant_final_condition(values):
    for value in values:
        try:
            callback = lookup(value)
        except KeyError:
            pass
        else:
            prepare(callback)
            if True:
                consume(value)
