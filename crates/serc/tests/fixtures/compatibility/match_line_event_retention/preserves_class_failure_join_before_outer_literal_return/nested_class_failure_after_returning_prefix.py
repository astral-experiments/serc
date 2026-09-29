def f(flag, subject, probe):
    if flag:
        if probe(subject):
            return False
        match subject:
            case Item(parent=Parent()):
                return True
    elif subject:
        return True
    return False
