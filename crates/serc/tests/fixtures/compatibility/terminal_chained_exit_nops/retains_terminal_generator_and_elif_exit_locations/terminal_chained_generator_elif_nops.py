def generator(low, value, high, ready):
    yield value
    if not (ready and low < value < high):
        return value

def final_elif(low, value, high, ready):
    if ready:
        work()
    elif not low < value < high:
        raise Error

def same_line_control(low, value, high):
    if not low < value < high: return value

def preceding_call_control(low, value, high):
    if not low < value < high:
        work()
        return value

def explicit_following_return_control(low, value, high):
    if not low < value < high:
        return value
    return low
