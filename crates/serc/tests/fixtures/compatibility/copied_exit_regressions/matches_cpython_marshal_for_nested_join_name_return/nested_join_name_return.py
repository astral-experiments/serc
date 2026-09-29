def nested_join_name_return(a, b, c, d, emit, value):
    if a:
        if b:
            if c:
                emit(1)
            else:
                emit(2)
        else:
            if d:
                emit(3)
            else:
                emit(4)
    else:
        emit(5)
    return value


def nested_join_name_return_deeper(a, b, c, d, e, emit, value):
    if a:
        if b:
            if c:
                emit(1)
            else:
                emit(2)
        else:
            if c:
                if d:
                    emit(3)
                else:
                    emit(4)
            elif e:
                emit(5)
            else:
                emit(6)
    else:
        emit(7)
    return value
