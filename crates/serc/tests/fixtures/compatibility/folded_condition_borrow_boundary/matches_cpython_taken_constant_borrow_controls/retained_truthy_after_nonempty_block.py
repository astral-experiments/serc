def scan(lines):
    found = False
    for line in lines:
        found = False
        if (
            'enabled'
            and 'second' in line
        ):
            found = True
    return found
