def should_skip(path, custom=None):
    if path.endswith('/skip') or (custom(path) if custom else False):
        return True
    return False
