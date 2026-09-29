def scan(lines):
    found = False
    for line in lines:
        if 'first' in line:
            found = True
        if 0 or 'second' in line:
            found = True
    return found
