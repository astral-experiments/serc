def run(worker, cleanup, retry):
    try:
        try:
            return worker()
        except LookupError as error:
            if retry:
                try:
                    return worker()
                except OSError:
                    pass
                finally:
                    cleanup.finish(missing_ok=True)
            return worker()
    finally:
        cleanup.close()
