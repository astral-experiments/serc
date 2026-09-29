def simple_final_if(flag, result):
    while True:
        step()
        if flag:
            break
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def final_if_with_fallthrough(result):
    while True:
        value = read()
        if value is None:
            break
        elif value is ignored:
            pass
        else:
            consume(value)
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result
