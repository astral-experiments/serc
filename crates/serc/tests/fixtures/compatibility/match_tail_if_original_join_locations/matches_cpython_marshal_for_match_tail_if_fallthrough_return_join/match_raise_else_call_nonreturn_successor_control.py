def f(subject, flag):
    match subject:
        case First():
            if flag:
                raise ValueError()
            else:
                observe(subject)
        case _:
            raise ValueError()
    finish(subject)
    return subject
