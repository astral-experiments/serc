def run(work, clean, report, finish):
    try:
        try:
            work()
        except Exception as error:
            try:
                report(error)
                return 2
            finally:
                errors, interrupt = clean(error)
                if errors:
                    report(errors)
                if interrupt is not None:
                    raise interrupt
    finally:
        finish()
