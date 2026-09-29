def managed_value(manager, primary, secondary, restore, remove):
    with manager():
        try:
            yield
        finally:
            if primary():
                restore()
            elif secondary():
                remove()
