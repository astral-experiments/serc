def recover(fetch, report, first, second):
    result = None
    try:
        result = fetch()
    except Exception:
        if first:
            if second:
                report()
            else:
                report()
        else:
            report()
    return result
