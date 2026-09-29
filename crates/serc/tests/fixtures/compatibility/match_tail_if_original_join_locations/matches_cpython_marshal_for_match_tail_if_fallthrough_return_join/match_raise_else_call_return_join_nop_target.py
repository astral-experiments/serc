def f(subject, flag):
    match subject:
        case First():
            prepare(subject)
            if flag:
                raise ValueError()
            else:
                observe(subject)
        case Second():
            if flag:
                raise ValueError()
            else:
                observe(subject)
        case _:
            raise ValueError()
    return subject
