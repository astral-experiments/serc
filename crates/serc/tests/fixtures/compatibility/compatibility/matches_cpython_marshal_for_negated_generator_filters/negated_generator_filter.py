def filtered(items):
    return (item for item in items if not predicate(item))
