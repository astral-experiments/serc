def expression_fallthrough(manager, condition):
    with manager:
        try:
            assert body()
        finally:
            if condition:
                cleanup()

def attribute_fallthrough(manager, resource):
    with manager:
        try:
            assert body()
        finally:
            if resource.enabled:
                cleanup()

def call_fallthrough(manager, resource):
    with manager:
        try:
            assert body()
        finally:
            if resource.enabled():
                cleanup()
