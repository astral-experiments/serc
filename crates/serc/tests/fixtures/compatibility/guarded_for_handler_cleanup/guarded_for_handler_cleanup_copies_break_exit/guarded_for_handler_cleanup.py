def convert(values, items):
    for value in values:
        try:
            found = lookup(value)
        except TypeError:
            found = None
            if value in unhashable or value in hashable:
                for item in items:
                    if item.value == value:
                        found = item
                        break
        consume(found)
