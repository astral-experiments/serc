def branches(value, left, right):
    if value < 0:
        result = left
    elif value == 0:
        result = value
    elif value < 10 and left:
        result = left + right
    else:
        result = right

    if left or right:
        result += 1

    if left and not right:
        result -= 1

    return result
