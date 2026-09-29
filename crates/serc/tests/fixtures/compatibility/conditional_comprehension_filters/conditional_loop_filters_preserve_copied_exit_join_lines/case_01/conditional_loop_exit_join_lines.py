def check_type(obj, checks):
    for check in checks:
        if isinstance(obj, check) if isinstance(check, type) else check(obj):
            return True
    return False
