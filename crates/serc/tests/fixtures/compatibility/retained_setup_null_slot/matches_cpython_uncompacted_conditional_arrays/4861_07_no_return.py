def f(items):
    try:
        results = []
        for item in items:
            if item:
                try:
                    results.append({"value": item.value if item.value else ""})
                except Exception:
                    continue
    except Exception:
        raise
