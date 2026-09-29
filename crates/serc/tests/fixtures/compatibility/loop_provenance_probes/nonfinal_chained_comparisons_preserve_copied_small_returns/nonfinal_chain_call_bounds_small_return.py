def target(rules, value, ignored, is_range, split):
    for rule in rules:
        if ignored(rule):
            continue
        if is_range(rule):
            low, high = split(rule)
            if int(low) <= value <= int(high):
                return True
        elif int(rule) == value:
            return True
    return False
