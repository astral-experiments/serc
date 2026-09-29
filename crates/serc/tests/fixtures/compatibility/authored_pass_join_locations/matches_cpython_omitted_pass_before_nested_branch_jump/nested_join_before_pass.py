def f(flag, inner, other):
    try:
        if flag == 1:
            first()
            if inner:
                if other:
                    recover()
                else:
                    accept()
            pass
        elif flag == 2:
            last()
            pass
        else:
            raise Error()
    except Error as error:
        caught(error)
    finally:
        cleanup()
    return flag
