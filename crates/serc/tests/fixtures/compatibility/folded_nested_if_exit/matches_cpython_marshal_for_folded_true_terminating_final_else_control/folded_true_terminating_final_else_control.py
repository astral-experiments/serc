def route(value):
    if True:
        if value == 'left':
            first()
        elif value == 'right':
            second()
        else:
            return value
    else:
        discarded()
    if value == 'after':
        finish()
    complete()
