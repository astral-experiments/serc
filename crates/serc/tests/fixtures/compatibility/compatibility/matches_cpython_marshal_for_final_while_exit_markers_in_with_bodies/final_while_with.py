def single_exit(manager, flag):
    with manager:
        while flag:
            step()
    outside()


def multiple_condition_exits(manager, first, second):
    with manager:
        while first and second:
            step()
    outside()


def chained_comparison(manager, low, value, high):
    with manager:
        while low < value < high:
            step()
    outside()


def conditional_expression(manager, condition, first, second):
    with manager:
        while first if condition else second:
            step()
    outside()
