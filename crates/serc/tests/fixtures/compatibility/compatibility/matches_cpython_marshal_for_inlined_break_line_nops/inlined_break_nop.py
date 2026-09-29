def conditional(flag):
    while True:
        if flag:
            break
        step()
    return value

def direct():
    while True:
        break
    return value

def same_line():
    while True: break
    return value

def same_line_after_expression():
    while True: step(); break
    return value

def loop_else(flag):
    while flag:
        if done():
            break
        step()
    else:
        fallback()
    return value

def large_exit(flag):
    while True:
        if flag:
            break
        step()
    first(); second(); third(); fourth(); fifth()
    return value

def implicit_return(flag):
    while True:
        if flag:
            break
        step()

def managed(manager, flag):
    while True:
        with manager:
            if flag:
                break
            step()
    return value

def iteration(items, flag):
    for item in items:
        if flag:
            break
    return value
