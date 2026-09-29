def f(value):
    match value:
        case {"pool": {"created": created, "name": name, "kind": kind, "label": label}, "pod": {"ready": bool() as ready}}:
            return created, name, kind, label, ready
    return None
