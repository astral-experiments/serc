def returning(dependencies):
    try:
        before = load()
        after = load()
    except Exception:
        return
    for dependency in dependencies:
        if dependency in after:
            consume(dependency, before)

def reraising(dependencies):
    try:
        before = load()
        after = load()
    except Exception:
        raise
    for dependency in dependencies:
        if dependency in after:
            consume(dependency, before)
