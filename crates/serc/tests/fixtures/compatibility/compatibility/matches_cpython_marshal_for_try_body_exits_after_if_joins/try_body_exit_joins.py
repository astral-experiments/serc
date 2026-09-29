def folded_false_else():
    try:
        if False:
            value = 1
        else:
            value = 2
    except:
        pass
    return None


def folded_false_elif_else():
    try:
        if False:
            value = 1
        elif False:
            value = 2
        else:
            value = 3
    except:
        pass
    return None


def folded_true_recursive_else():
    try:
        if True:
            if False:
                value = 1
            else:
                value = 2
        else:
            value = 3
    except:
        pass
    return None


def folded_false_terminal_else():
    try:
        if False:
            value = 1
        else:
            return None
    except:
        pass
    return None


def folded_true_simple_else():
    try:
        if True:
            value = 1
        else:
            value = 2
    except:
        pass
    return None
