def f(values, left, right, accept):
    for value in values:
        match value:
            case 0:
                return False
            case 1:
                if left and right:
                    if accept(value):
                        return True
    return False
