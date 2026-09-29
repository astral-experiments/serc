def f(manager):
    total = 0
    try:
        with manager as stream:
            for item in stream:
                try:
                    value = item()
                except Exception:
                    continue
                if isinstance(value, dict):
                    choices = value.get("metadata", {}).get("choices")
                    if choices:
                        if isinstance(choices, list):
                            total += len(choices)
                        elif isinstance(choices, dict):
                            total += len(choices)
                        else:
                            total += 1
                    else:
                        total += 1
    except Exception:
        pass
    return total
