def managed_value(manager, primary, secondary, restore, remove, recover):
    with manager():
        try:
            yield
        except ValueError:
            if primary():
                recover()
        finally:
            if primary():
                restore()
            elif secondary():
                remove()
