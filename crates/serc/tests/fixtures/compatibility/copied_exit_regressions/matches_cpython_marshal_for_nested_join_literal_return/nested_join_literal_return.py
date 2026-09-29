def nested_join_literal_return(a, b, c, d, emit):
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
    return None


def nested_join_literal_return_deeper(a, b, c, d, e, emit):
    if a:
        if b:
            if c:
                emit(1)
            elif d:
                emit(2)
            else:
                emit(3)
        else:
            if c:
                if d:
                    emit(4)
                else:
                    emit(5)
            elif e:
                emit(6)
            else:
                emit(7)
    else:
        emit(8)
    return None
