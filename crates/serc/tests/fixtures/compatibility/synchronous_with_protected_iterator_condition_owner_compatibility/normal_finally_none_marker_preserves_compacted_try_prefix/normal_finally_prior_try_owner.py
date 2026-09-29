def f(manager, flag, first, second, third):
    def capture():
        return flag
    with manager:
        try:
            left = int(first.value)
            middle = int(second.value)
            right = int(third.value)
        finally:
            if flag is not None:
                flag.close()
    return left, middle, right, capture
