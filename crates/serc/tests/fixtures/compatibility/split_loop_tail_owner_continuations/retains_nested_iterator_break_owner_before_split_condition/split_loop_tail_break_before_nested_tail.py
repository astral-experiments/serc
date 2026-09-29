def contains_match(records, accept):
    found = False
    failed = False
    for record in records:
        try:
            values = record.values
            if isinstance(values, list):
                for value in values:
                    if accept(value):
                        found = True
                        break
            if not found:
                for key in ("one", "two", "three", "four", "five"):
                    value = getattr(record, key, None)
                    if accept(value):
                        found = True
                        break
            if not found:
                if accept(record.fallback):
                    found = True
        except Exception:
            failed = True
    return found, failed
