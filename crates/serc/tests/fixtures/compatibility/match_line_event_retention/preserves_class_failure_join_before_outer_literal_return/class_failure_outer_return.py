def f(flag, subject):
    if flag:
        match subject:
            case Item():
                return True
    elif subject:
        return True
    return False
