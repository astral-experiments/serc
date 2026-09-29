def f(expected, actual):
    i = j = 0
    while True:
        try:
            left = expected[i]
            right = actual[j]
            if left < right:
                i += 1
                while expected[i] == left:
                    i += 1
            elif left > right:
                j += 1
                while actual[j] == right:
                    j += 1
            else:
                i += 1
                j += 1
        except IndexError:
            break
    return i, j
