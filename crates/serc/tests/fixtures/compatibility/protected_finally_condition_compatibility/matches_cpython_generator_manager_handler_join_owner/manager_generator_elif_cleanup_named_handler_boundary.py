def managed_value(manager, primary, secondary, restore, remove, recover):
    with manager():
        try:
            yield
        except ValueError as error:
            recover(error)
        finally:
            if primary():
                restore()
            elif secondary():
                remove()
