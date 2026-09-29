def f(values, flag, other):
    for value in values:
        try:
            work(value)
        except Error as error:
            assert flag or other
        else:
            raise Other
