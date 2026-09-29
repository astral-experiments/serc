def run(work, cleanup, report, fallback):
    try:
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
    except Exception:
        return 0
    return 0
