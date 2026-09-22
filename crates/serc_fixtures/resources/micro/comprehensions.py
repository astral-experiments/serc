def comprehensions(items, groups):
    listed = [item * 2 for item in items if item > 0]
    mapped = {item: item * 2 for item in items if item > 0}
    unique = {item * 2 for item in items if item > 0}
    generated = (item * 2 for item in items if item > 0)
    nested = [(left, right) for left in groups for right in groups[left] if right != left]
    assigned = [value for item in items if (value := normalize(item)) is not None]
    return listed, mapped, unique, generated, nested, assigned, value
