def scan(lines, enabled):
    found = False
    for line in lines:
        if 'first' in line:
            found = True
        if enabled and 'second' in line:
            found = True
    return found
