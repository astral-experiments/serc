def should_skip(path, custom=None):
    if path.endswith('/skip') or (custom and custom(path)):
        return True
    return False
