def run(outer, retained, choose, value, report):
    outcome = None
    complete = False
    with outer, retained:
        try:
            if choose:
                complete = True
                logger.info("completed")
                return value
            return value
        except Exception:
            raise
        finally:
            if outcome is not None or not complete:
                report(outcome)
