def run(outer, retained, inner, choose, report):
    outcome = None
    result = None
    complete = False
    details = None
    with outer, retained:
        try:
            with inner:
                try:
                    operation()
                except Exception as error:
                    outcome = 1 if choose else 2
                    details = {
                        "reason": "retry",
                        "exception_type": type(error).__name__,
                        "exception_message": str(error),
                    }
                    return build_result(value=None)
        except Exception:
            raise
        finally:
            if outcome is not None or (result is None and not complete):
                report(details)
