async def managed_value(manager, operation, primary, secondary, restore, remove):
    with manager():
        try:
            await operation()
        finally:
            if primary():
                restore()
            elif secondary():
                remove()
