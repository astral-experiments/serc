def run(manager, cleanup, work, report):
    with manager:
        try:
            work()
        except LookupError as error:
            report(error)
            if cleanup is None:
                raise ValueError("missing") from error
            raise RuntimeError("lookup") from error
        except ValueError as error:
            report(error)
            raise RuntimeError("invalid") from error
        except Exception as error:
            raise RuntimeError("failed") from error
        finally:
            if cleanup is not None:
                cleanup.close()
                report("closed")
