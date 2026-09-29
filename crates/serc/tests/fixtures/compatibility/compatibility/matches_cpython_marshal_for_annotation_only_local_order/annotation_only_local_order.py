def tuple_target(values):
    annotated: object
    first, annotated = values
    return first, annotated


def branch_target(condition):
    later: object
    earlier: object
    if condition:
        middle = 1
        earlier = 2
    else:
        later = 3
    return middle, earlier, later


def comprehension_target(values):
    result: list
    result = [item for item in values]
    return result


def annotated_comprehension_target(values):
    item: object
    first = 1
    result = [item for item in values]
    return first, result


def definition_target():
    callback: object
    prior = 1

    def callback():
        pass

    return prior, callback


def captured_only():
    captured: object
    first = 1

    def inner():
        return captured

    return first, inner


def read_target():
    annotated: object
    return annotated


def self_referencing_target():
    target: object
    rhs: object
    first = 1
    target = rhs + target


def augmented_target():
    target: object
    rhs: object
    first = 1
    target += rhs


def stored_subscript_target():
    container: object
    index: object
    rhs: object
    first = 1
    container[index] = rhs


def augmented_subscript_target():
    container: object
    index: object
    rhs: object
    first = 1
    container[index] += rhs


def annotated_attribute_target():
    container: object
    first = 1
    container.value: object


def annotated_subscript_target():
    container: object
    index: object
    first = 1
    container[index]: object


def deleted_subscript_target():
    container: object
    index: object
    first = 1
    del container[index]


def loop_attribute_target(values):
    container: object
    first = 1
    for container.value in values:
        pass


def context_attribute_target(context):
    container: object
    first = 1
    with context as container.value:
        pass


def comprehension_attribute_target(values):
    container: object
    first = 1
    result = [container.value for container.value in values]
    return first, result


def complex_comprehension_order():
    source: object
    container: object
    first = 1
    result = [0 for container.value in source]
    return first, result


def mixed_comprehension_order(values):
    container: object
    index: object
    named: object
    first = 1
    result = [named for container[index], named in values]
    return first, result


def comprehension_source_order():
    source: object
    item: object
    first = 1
    result = [item for item in source]
    return first, result


def comprehension_outer_iterable_order():
    earlier: object
    target: object
    first = 1
    target = [item for item in (earlier, target)]
    return target


def nested_comprehension_order(outer):
    result = [[nested for nested in inner] for item in outer]
    inner = ()
    return result


def exception_target():
    exception: object
    exception_type: object
    first = 1
    try:
        pass
    except exception_type as exception:
        pass


def lambda_default():
    defaulted: object
    first = 1
    callback = lambda value=defaulted: value
    return callback


def generator_iterable():
    iterable: object
    first = 1
    generated = (value for value in iterable)
    return generated
