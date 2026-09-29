def protected(manager, flag):
    with manager:
        try:
            action()
        except Error:
            result = check()
            if result:
                report(result)


def explicit_else_control(flag):
    try:
        action()
    except Error:
        first()
        if flag:
            second()
        else:
            third()
