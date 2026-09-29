def f(condition, index, limit):
    if condition:
        value = 1
        while index < limit:
            index += 1
    else:
        fallback = 1
        while index > 0:
            index -= 1
    tail0 = 0
    tail1 = 1
    tail2 = 2
    tail3 = 3
    tail4 = 4
