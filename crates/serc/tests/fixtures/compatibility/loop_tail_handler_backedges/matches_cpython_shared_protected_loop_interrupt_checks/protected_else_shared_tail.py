def f(items):
    for item in items:
        try:
            if item is None:
                continue
            else:
                consume(item)
                if ready(item):
                    emit(item)
        except Exception as exc:
            recover(exc)
