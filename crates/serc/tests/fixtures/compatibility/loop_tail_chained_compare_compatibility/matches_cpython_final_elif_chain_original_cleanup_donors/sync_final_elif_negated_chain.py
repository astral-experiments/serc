def run(items, upper, output):
    for value in items:
        if value is None:
            output.append(value)
        elif not 0 <= value <= upper:
            output.append(value)
    return output
