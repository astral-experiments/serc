try:
    work()
except Error:
    if 1:
        if 2:
            pass
        else:
            raise InnerError
    else:
        raise OuterError
finish()
