def f(manager, items, limit=None):
    count = 0
    seen = 0
    errors = 0
    with manager as stream:
        for item in items:
            if limit is not None and seen >= limit:
                break
            seen += 1
            try:
                result = process(item)
                if result is None:
                    skipped(item)
                    errors += 1
                    continue
                else:
                    stream.write(result)
                    count += 1
                    if count % 100 == 0:
                        report(count)
            except Exception as exc:
                recover(exc)
    complete(errors)
