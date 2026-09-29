def visit(resources, perform):
    count = 0
    for resource in resources:
        try:
            if perform(resource):
                continue
            count += 1
        finally:
            if resource is not None:
                try:
                    resource.close()
                except Exception:
                    pass
    return count
