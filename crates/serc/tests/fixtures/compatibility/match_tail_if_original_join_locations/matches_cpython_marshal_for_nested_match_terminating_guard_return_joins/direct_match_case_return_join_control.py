def f(subject, flag):
    match subject:
        case Inner():
            if flag:
                return False
        case None:
            pass
        case _:
            observe(subject)
    return True
