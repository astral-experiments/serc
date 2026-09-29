def run(outer, retained, choose, report):
    outcome = None
    result = None
    complete = False
    with outer, retained:
        try:
            if choose:
                complete = True
                logger.info("completed")
                return build_result(
                    value=None,
                    complete=True,
                )
            return build_result(value=None, complete=True)
        except Exception:
            raise
        finally:
            if outcome is not None or (result is None and not complete):
                report(outcome)
