def update(target, source, force):
    for key in source:
        if key == "first":
            if force or key not in target:
                target[key] = source[key]
        elif key == "second":
            target[key] = source[key]
        else:
            raise ValueError(key)
