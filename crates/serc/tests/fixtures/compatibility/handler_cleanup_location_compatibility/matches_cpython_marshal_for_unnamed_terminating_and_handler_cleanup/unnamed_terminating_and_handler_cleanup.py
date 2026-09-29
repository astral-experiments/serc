def calendar_shape(expanded, value, previous):
    try:
        expanded.index(value)
    except ValueError:
        difference = nearest(value, expanded)
        if difference is not None and difference != 0:
            if previous:
                value = rewind(value, difference)
            else:
                value = advance(value, difference)
            return True, value
    return False, value


def inherited_return_shape(value, name, environment):
    if value:
        try:
            return construct(getattr(value, name), environment)
        except AttributeError:
            if is_name(value) and value.id == name:
                return value
    raise ValueError(name)


def inherited_raise_shape(value, labels, axis):
    if axis:
        try:
            return convert(value, labels)
        except KeyError:
            if is_scalar(value) and is_valid(value):
                if axis == 1:
                    return labels
                raise ValueError(value)
        except TypeError:
            recover(value)
        except ValueError:
            recover(labels)
    return labels


def three_term_and_shape(first, second, third, value):
    try:
        action()
    except Error:
        if first and second and third:
            return value
    after()


def false_and_value_control(value):
    try:
        action()
    except Error:
        if False and value:
            return value
    after()


def value_and_false_control(value):
    try:
        action()
    except Error:
        if value and False:
            return value
    after()


def simple_condition_control(first, value):
    try:
        action()
    except Error:
        if first:
            return value
    after()
