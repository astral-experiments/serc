def break_not_retained(values):
    for value in values:
        try:
            if value:
                break
        except KeyError:
            break
    return None
