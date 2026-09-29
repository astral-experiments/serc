def run(groups, ready, consume):
    for group in groups:
        try:
            iterator = iter(group)
            while ready(iterator):
                consume(next(iterator))
        except Exception:
            pass
