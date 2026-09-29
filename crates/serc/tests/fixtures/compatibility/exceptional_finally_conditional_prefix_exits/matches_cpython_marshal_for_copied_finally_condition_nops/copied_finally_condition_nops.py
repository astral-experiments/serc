def boolean_condition(manager, left, right):
    with manager:
        try:
            assert body()
        finally:
            if left and right:
                cleanup()


def conditional_expression(manager, condition, left, right):
    with manager:
        try:
            assert body()
        finally:
            if left if condition else right:
                cleanup()


def negated_boolean_condition(manager, left, right):
    with manager:
        try:
            assert body()
        finally:
            if not (left or right):
                cleanup()
