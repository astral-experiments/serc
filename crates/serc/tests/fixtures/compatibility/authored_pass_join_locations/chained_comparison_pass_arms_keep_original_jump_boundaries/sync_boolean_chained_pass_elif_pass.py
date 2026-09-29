def run(skip_empty, skip_busy, completed, failed, busy, report):
    if skip_empty and completed == failed == 0:
        pass
    elif skip_busy and not busy:
        pass
    else:
        report()
    return completed, failed
