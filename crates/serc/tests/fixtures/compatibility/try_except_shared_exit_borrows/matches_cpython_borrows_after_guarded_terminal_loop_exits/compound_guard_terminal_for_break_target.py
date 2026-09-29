def f(data, check, load, report):
    result = None
    try:
        entries = load(data)
        report(entries)
        if data and check(data):
            for entry in entries:
                try:
                    row = load(entry)
                except Exception:
                    continue
                if check(row):
                    if row.ready:
                        return {"row": row, "data": data, "ready": True}
                    result = load(row)
                    if result.ready:
                        result.value = data
                        return result
                    break
    except Exception:
        pass
    result = load(data)
    report(data, result)
    if check(result):
        result = load(result)
    report(data, result)
    return result
