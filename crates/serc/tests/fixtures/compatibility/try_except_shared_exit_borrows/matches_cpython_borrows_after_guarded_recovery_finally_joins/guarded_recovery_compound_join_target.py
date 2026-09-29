def collect(resource, work, report, enabled, option, retry):
    try:
        try:
            value = work(resource, option=option if enabled else None)
            if value.failed:
                raise ValueError("failed")
            report(value)
            errors = []
            if enabled and option:
                errors = [{"error": "incomplete"}]
        except Exception as error:
            if not retry:
                raise
            errors = [{"error": str(error)}]
    finally:
        resource.close(missing_ok=True)
    report(resource, errors)
    if errors:
        report(errors)
    return errors
