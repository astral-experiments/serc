def run(work, clean, report, finish, resource):
    try:
        try:
            try:
                return work()
            finally:
                if resource is not None:
                    errors, interrupt = clean(resource)
                    if errors:
                        report(errors)
                    if interrupt is not None:
                        raise interrupt
        except Exception:
            raise
    finally:
        finish()
