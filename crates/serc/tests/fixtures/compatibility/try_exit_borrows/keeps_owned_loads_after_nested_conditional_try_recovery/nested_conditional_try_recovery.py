def f(items, load, parse):
    total = 0
    out = {}
    for key in items:
        try:
            record = load(key)
            good = int(record.good or 0)
            bad = int(record.bad or 0)
            if good == 0 and bad == 0:
                try:
                    other = parse(key)
                    if isinstance(other, dict) and "count" in other:
                        good = int(other.get("count") or 0)
                except Exception:
                    pass
        except Exception:
            good = 0
            bad = 0
        out[key] = (good, bad)
        total += good
    return out, total
