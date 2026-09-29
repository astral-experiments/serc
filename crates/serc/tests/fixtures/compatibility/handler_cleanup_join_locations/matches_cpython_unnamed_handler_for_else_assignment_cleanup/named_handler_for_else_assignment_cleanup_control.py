def lookup(mapping, key):
    try:
        value = mapping[key]
    except KeyError as error:
        for candidate, replacement in mapping.items():
            if candidate.lower() == key.lower():
                value = replacement
                break
        else:
            value = range(8)
    return value.start, value.stop
