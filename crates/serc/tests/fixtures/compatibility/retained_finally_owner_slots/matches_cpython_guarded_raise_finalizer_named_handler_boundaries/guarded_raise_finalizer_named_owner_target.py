def run(work, clean, report, finish):
    try:
        try:
            work()
        except Exception as error:
            try:
                return work(error)
            finally:
                errors, interrupt = clean(error)
                if errors:
                    report(errors)
                if interrupt is not None:
                    raise interrupt
    finally:
        finish()
