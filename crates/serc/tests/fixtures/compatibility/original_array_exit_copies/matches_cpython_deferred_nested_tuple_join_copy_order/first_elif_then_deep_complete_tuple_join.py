def choose(category, mode, accepted, action, failed, subtype, other):
    if category == 1:
        if mode:
            result = 1
        elif accepted:
            result = 2
        else:
            result = 3
    elif category == 2:
        if mode:
            if accepted:
                result = 2
            else:
                if action:
                    result = 3
                else:
                    if failed:
                        if subtype:
                            result = 4
                        else:
                            result = 5
                    else:
                        result = 6
        else:
            if accepted:
                result = 7
            else:
                if action:
                    result = 8
                else:
                    result = 9
    elif category == 3:
        return None, None
    else:
        raise ValueError(category)
    return result, other
