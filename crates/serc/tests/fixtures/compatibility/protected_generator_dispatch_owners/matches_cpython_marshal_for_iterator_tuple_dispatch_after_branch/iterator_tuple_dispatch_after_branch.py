def after_branch(groups, ready):
    try:
        for values in groups:
            if ready:
                raise Error
            result = tuple(value for value in values)
            consume(result)
    except Error:
        pass


def first_iterator_slot_control(groups):
    try:
        for values in groups:
            result = tuple(value for value in values)
            consume(result)
    except Error:
        pass
