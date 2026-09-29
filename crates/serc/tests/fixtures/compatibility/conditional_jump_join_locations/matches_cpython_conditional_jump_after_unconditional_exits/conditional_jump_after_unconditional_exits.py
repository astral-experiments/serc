def finalizer(outer, inner, call):
    if outer:
        if inner:
            before()
            try:
                call()
            finally:
                after()
    else:
        alternate()
    finish()

def exhaustive_match(outer, inner, mode, value):
    if outer:
        if inner:
            match mode:
                case None:
                    raise ValueError
                case 0:
                    value = inner
                case _:
                    raise TypeError
    else:
        value = outer
    finish(value)

def following_work(outer, inner, mode, value):
    if outer:
        if inner:
            match mode:
                case None:
                    raise ValueError
                case 0:
                    value = inner
                case _:
                    raise TypeError
            consume(value)
    else:
        value = outer
    finish(value)
