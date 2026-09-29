def straight():
    a = -1.0
    b = not 1
    c = not ""
    return a, b, c

def reversed():
    a = -1.0
    b = not ""
    c = not 1
    return a, b, c

def literal_first():
    a = False
    b = -1.0
    c = not 1
    d = not ""
    return a, b, c, d

def nested():
    a = -1.0
    b = (not 1, not "", not 2)
    return a, b

def linear():
    return (-1.0, not 1, not "")
