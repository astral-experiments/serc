def run(outer, inner, cleanup, factory, value, report):
    with outer:
        try:
            with inner:
                return factory(normalize(value))
        except LookupError as error:
            raise RuntimeError("lookup") from error
        except ValueError as error:
            raise RuntimeError("invalid") from error
        except Exception as error:
            raise RuntimeError("failed") from error
        finally:
            if cleanup is not None:
                cleanup.close()
                report("closed")
