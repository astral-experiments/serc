def early_return_nop_owner(resource):
    try:
        consume(resource)
        consume(resource)
        consume(resource)
        return 0
    finally:
        if resource is not None:
            finish(resource)
