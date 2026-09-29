def run(groups, ready, remaining, consume):
    for group in groups:
        try:
            iterator = iter(group)
            while ready(iterator) and remaining():
                consume(next(iterator))
        except Exception:
            pass
