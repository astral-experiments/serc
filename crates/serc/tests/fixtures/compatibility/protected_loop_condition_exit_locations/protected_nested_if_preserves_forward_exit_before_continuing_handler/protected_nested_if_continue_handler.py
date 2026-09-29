def probe(items, outer, inner, work, report):
    for item in items:
        try:
            if outer(item):
                if inner(item):
                    work(item)
        except Exception:
            report(item)
            continue
