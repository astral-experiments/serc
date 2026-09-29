def collect(items, left, middle, right, output):
    for value in items:
        if value == '(':
            left += 1
        elif value == ')':
            left -= 1
        elif value == ',' and left == middle == right == 0:
            output.append(value)
    return output
