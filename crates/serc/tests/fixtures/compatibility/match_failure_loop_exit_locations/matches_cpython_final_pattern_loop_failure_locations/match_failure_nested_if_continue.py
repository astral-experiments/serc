def f(xs, flag, other):
    for x in xs:
        if flag:
            if other:
                match x:
                    case 0:
                        return x
                    case None:
                        continue
            else:
                continue
    return None
