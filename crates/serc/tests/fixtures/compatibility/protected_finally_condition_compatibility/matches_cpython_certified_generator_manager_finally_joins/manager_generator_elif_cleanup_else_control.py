def managed_value(manager, primary, secondary, restore, remove, fallback):
    with manager():
        try:
            yield
        finally:
            if primary():
                restore()
            elif secondary():
                remove()
            else:
                fallback()
