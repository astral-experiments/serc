def f(items, action, record):
    for item in items:
        try:
            begin(item)
            try:
                action(item)
            except IOError as exc:
                report(exc)
                if item:
                    record(item)
            except Exception as exc:
                report(exc)
                if item:
                    record(None)
        except Exception as exc:
            report(exc)
            if item:
                record(item)
    return items
