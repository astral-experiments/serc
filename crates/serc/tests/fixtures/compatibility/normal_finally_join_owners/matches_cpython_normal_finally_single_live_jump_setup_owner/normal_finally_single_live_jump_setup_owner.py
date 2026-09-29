def complete(manager, condition, exists, remove, report):
    with manager():
        try:
            if condition():
                raise ValueError("invalid")
        except Exception:
            report()
            raise
        finally:
            try:
                if exists():
                    remove()
            except Exception:
                report()
