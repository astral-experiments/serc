def probe(outer, middle, inner, raw):
    value = None
    tag = "missing"
    try:
        if outer is not None:
            if middle is not None:
                if inner is not None:
                    if isinstance(raw, str) and raw:
                        value = raw
                        tag = raw
                    else:
                        tag = "empty"
                else:
                    tag = "empty"
            else:
                tag = "no_middle"
    except Exception:
        tag = "error"
    return value, tag
