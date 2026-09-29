def test(outer, inner):
    try:
        work()
    except Error as error:
        if outer:
            first(error)
        else:
            if inner:
                second()
            else:
                third()
