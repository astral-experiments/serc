def f(items, report):
    for value in items:
        try:
            risky(value)
        except Exception as exc:
            if value is not None:
                report(exc)
        checkpoint()
