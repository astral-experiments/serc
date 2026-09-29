def example(manager, values, flag):
    with manager:
        if flag:
            for value in values:
                consume(value)
        else:
            raise Error
    finish()
