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
                    try:
                        result = write(path)
                    except Exception as error:
                        result = str(error)
    else:
        report("other mode")
    report(result)
