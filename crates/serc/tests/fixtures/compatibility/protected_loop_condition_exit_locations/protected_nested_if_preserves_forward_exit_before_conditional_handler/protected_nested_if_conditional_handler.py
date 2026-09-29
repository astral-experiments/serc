def probe(items, outer, inner, work, recover, report):
    for item in items:
        try:
            if outer(item):
                if inner(item):
                    work(item)
        except Exception:
            if recover(item):
                report(item)
