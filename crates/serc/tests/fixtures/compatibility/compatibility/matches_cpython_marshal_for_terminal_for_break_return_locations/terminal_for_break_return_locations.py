def multiple_breaks(items, flag):
    if flag:
        for item in items:
            if first(item):
                break
            if second(item):
                work(item)
                break
    else:
        alternate()

def for_else(items, flag):
    if flag:
        for item in items:
            if first(item):
                work(item)
                break
        else:
            exhausted()
    else:
        alternate()

def break_then_fallthrough(items, flag):
    if flag:
        for item in items:
            if first(item):
                break
            work(item)
    else:
        alternate()

def nested_break_control(items, flag):
    if flag:
        for item in items:
            while ready(item):
                break
            work(item)
    else:
        alternate()

def no_break_control(items, flag):
    if flag:
        for item in items:
            work(item)
    else:
        alternate()

def guarded_match_break(items, flag):
    if flag:
        for item in items:
            match item:
                case _ if ready(item):
                    break
                case _:
                    work(item)
    else:
        alternate()

def direct_loop_control(items):
    for item in items:
        if first(item):
            work(item)
            break
