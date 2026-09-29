deps = [item for item in items if item]
try:
    if isinstance(value, str) and value == "Auto":
        result = None
    else:
        result = min(int(value), 30)
except Exception:
    result = None
consume(result, deps)
