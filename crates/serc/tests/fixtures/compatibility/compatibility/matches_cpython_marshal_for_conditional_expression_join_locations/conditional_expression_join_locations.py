def split_short_circuit(manager, value):
    manager.first = (value.items or []) if has_value(
        value, 'items') else []
    manager.second = (value.other or []) if has_value(
        value, 'other') else []


def nested_body(outer, inner, first, second, third):
    value = (first if inner else second) if outer else third
    consume(value)


def recursive_nested_body(outer, middle, inner, first, second, third, fourth):
    value = ((first if inner else second) if middle else third) if outer else fourth
    consume(value)


def folded_true_nested_body(outer, first, second, third):
    value = (first if True else second) if outer else third
    consume(value)


def folded_false_nested_body(outer, first, second, third):
    value = (first if False else second) if outer else third
    consume(value)


def nested_short_circuit_fallthrough(outer, inner, first, second, third, fourth):
    value = (first if inner else (second or third)) if outer else fourth
    consume(value)


def nested_else_control(outer, inner, first, second, third):
    value = first if outer else (second if inner else third)
    consume(value)


def return_control(outer, inner, first, second, third):
    return (first if inner else second) if outer else third


def same_block_control(condition, body, otherwise):
    return body if condition else otherwise


def call_control(condition):
    return body() if condition else otherwise()
