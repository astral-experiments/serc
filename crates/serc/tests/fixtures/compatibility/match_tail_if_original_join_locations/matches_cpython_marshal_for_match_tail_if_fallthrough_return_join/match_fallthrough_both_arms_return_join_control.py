def f(subject, flag):
    match subject:
        case First():
            if flag:
                prepare(subject)
            else:
                observe(subject)
        case _:
            raise ValueError()
    return subject
