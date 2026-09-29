def f(values, left, right):
    result = None
    for value in values:
        match value:
            case 1:
                if left and right:
                    result = value
                    left = False
                    right = True
    return result
