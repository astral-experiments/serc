def scan(lines):
    found = False
    for line in lines:
        if 'first' in line:
            found = True
        if 1 and 'second' in line:
            found = True
    return found
