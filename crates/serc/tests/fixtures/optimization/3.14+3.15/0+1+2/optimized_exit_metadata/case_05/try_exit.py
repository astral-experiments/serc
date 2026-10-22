def example(values, x, y):
    for item in values:
        try:
            if x:
                a()
            elif y:
                b()
            else:
                assert False
        except Exception as error:
            c(error)
            continue
        after()
