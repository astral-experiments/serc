def collect(mode, take):
    count = 0
    def below_limit():
        return count < 4
    if mode:
        while below_limit():
            value = take()
            count += 1
            if value is None:
                break
    else:
        count = 1
    return count
