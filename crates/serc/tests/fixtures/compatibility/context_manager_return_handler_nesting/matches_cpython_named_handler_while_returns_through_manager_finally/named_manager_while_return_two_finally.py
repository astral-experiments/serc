def worker(manager, ready, read, report, process):
    with manager:
        try:
            try:
                while ready():
                    try:
                        read()
                    except LookupError as error:
                        report(error)
                        return
            except ValueError as error:
                report(error)
            finally:
                try:
                    report(None)
                    while read():
                        pass
                except OSError as error:
                    report(error)
                report(None)
                if process is not None:
                    process.close()
        except RuntimeError as error:
            report(error)
            report(None)
            report(error)
        finally:
            report(None)
            if ready():
                report(None)
