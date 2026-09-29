def discarded_and():
    unused = 0 and not 1
    a = not ""
    b = not 1
    return a, b

def discarded_or():
    unused = 1 or not ""
    a = not 1
    b = not ""
    return a, b

def discarded_arm():
    unused = not 1 if 0 else 5
    a = not ""
    b = not 1
    return a, b
