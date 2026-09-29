def render(mode, enabled, first, second, prepare, write, report):
    result = None
    if mode:
        if enabled():
            if not first:
                result = "first missing"
            elif not second:
                result = "second missing"
            else:
                path = prepare()
                if not path:
                    result = "path missing"
                else:
                    result = write(path)
    else:
        report("other mode")
    report(result)
