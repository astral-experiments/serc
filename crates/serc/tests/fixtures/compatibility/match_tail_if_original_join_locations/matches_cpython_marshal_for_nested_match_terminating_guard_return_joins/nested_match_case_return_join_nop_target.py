def f(subject, inner, flag):
    match subject:
        case Outer():
            match inner:
                case Inner():
                    if flag:
                        return False
                case None:
                    pass
                case _:
                    observe(inner)
        case _:
            raise ValueError()
    return True
