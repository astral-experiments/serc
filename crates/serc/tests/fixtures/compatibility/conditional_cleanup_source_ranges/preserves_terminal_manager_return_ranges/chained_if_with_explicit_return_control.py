def inspect(low, value, high, manager):
    if low <= value < high:
        with manager():
            return value
