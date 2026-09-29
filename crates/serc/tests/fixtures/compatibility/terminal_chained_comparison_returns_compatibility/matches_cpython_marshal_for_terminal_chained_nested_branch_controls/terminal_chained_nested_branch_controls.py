def nested_if(low, value, high, ready):
    if low < value < high:
        if ready:
            emit(value)

def nested_else(low, value, high, ready):
    if low < value < high:
        if ready:
            emit(
                value,
            )
        else:
            emit(high)

def folded_nested_if(low, value, high):
    if low < value < high:
        if True:
            emit(value)

def assignment_control(low, value, high):
    if low < value < high:
        result = value

def nonterminal_control(low, value, high, ready):
    if low < value < high:
        if ready:
            emit(value)
    emit(high)
