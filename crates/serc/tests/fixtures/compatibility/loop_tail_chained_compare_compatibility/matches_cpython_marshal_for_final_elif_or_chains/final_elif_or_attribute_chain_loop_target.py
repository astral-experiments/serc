def collect(items, bounds, output):
    count = 0
    for value in items:
        if value == bounds.stop:
            count += 1
        elif count == 0:
            if value == bounds.special or bounds.low <= value <= bounds.high:
                output.append(value)
            elif value < bounds.limit:
                output.append(value)
        elif value == bounds.special or bounds.low <= value <= bounds.high:
            output.append(value)
    return count, output
