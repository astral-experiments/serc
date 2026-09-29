def run(a, b, c, fallback):
    if a < b < c:
        pass
    else:
        fallback()
    return None
