def nested_else_compare(value):
    return "positive" if value > 0 else "negative" if value < 0 else "zero"


def nested_else_depth_four(value, first, second, third, fourth, last):
    return first if value > 3 else second if value > 2 else third if value > 1 else fourth if value > 0 else last


def nested_else_name(value, positive, negative, zero):
    return positive if value > 0 else negative if value < 0 else zero


def nested_else_attribute(value, result):
    return result.positive if value > 0 else result.negative if value < 0 else result.zero


def nested_else_call(value, result):
    return result("positive") if value > 0 else result("negative") if value < 0 else result("zero")


def nested_else_boolean_tests(value, first, second, last, left, right):
    return first if left and value > 0 else second if right or value < 0 else last


def nested_else_named_tests(value, first, second, last, check):
    return first if (value := check(value, 1)) else second if (value := check(value, 2)) else last


def nested_else_comprehension_leaves(value, values, fallback):
    return [item for item in values if item] if value > 0 else {item for item in values if item} if value < 0 else fallback


def nested_else_branch(value, enabled, positive, negative, zero):
    if enabled:
        return positive if value > 0 else negative if value < 0 else zero
    return zero


def nested_else_closure(value, positive, negative, zero):
    def inner():
        return positive if value > 0 else negative if value < 0 else zero

    return inner()


def nested_body_control(value):
    return ("positive" if value > 1 else "small") if value > 0 else "negative"


def single_conditional_control(value, positive, zero):
    return positive if value > 0 else zero


def nested_loop_control(values, others, positive, negative, zero):
    for value in values:
        for other in others:
            return positive if value > other else negative if value < other else zero
    return zero


def with_control(value, manager, positive, negative, zero):
    with manager:
        return positive if value > 0 else negative if value < 0 else zero


def nested_with_control(value, first, second, positive, negative, zero):
    with first:
        with second:
            return positive if value > 0 else negative if value < 0 else zero


def inner_true_control(value, positive, negative, zero):
    return positive if value > 0 else negative if True else zero


def inner_false_control(value, positive, negative, zero):
    return positive if value > 0 else negative if False else zero


def inner_not_zero_control(value, positive, negative, zero):
    return positive if value > 0 else negative if not 0 else zero


def inner_ellipsis_control(value, positive, negative, zero):
    return positive if value > 0 else negative if ... else zero


def inner_debug_control(value, positive, negative, zero):
    return positive if value > 0 else negative if __debug__ else zero
