def run(work, cleanup, report, fallback):
    try:
        work()
        try:
            cleanup()
        except LookupError:
            pass
    except ValueError:
        try:
            report()
        except RuntimeError:
            fallback()
    return 0
