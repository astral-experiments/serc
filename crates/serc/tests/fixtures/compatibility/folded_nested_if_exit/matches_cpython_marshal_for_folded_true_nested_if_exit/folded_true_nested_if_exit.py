def route(value):
    if True:
        prepare()
        if value == 'left':
            first()
        elif value == 'right':
            second()
        else:
            third()
    else:
        discarded()
    if value == 'after':
        finish()
    complete()
