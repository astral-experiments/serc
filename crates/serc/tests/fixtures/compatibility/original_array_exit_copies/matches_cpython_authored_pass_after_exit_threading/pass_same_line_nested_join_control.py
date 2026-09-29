def probe(outer, inner, keep, other):
    if outer:
        if inner: pass
        else:
            if keep:
                return 1
    elif other:
        return 2
    return None
