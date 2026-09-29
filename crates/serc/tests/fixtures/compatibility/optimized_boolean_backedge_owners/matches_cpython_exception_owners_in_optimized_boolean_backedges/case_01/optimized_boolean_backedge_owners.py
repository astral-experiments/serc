def f(groups):
    try:
        return [group for group in groups if any(not item.disabled for item in group)]
    except Error:
        recover()
