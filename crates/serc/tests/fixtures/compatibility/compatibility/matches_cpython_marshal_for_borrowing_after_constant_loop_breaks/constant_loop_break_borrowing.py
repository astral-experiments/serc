def unprotected(flag, result):
    while True:
        if flag:
            break
        step()
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def copied_tiny_exit(flag, result):
    while True:
        step()
        if flag:
            break
    return result

def generator(flag, result):
    while True:
        if flag:
            break
        yield step()
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def tail_break(flag, result):
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

def nested_terminal_break(flag, result, next_item, stop):
    while True:
        outer = next_item()
        if outer is None:
            break
        if flag:
            while True:
                item = next_item()
                if item is None:
                    break
                consume(item)
                if stop(item):
                    break
        first(result)
        second(result)
        third(result)
        fourth(result)
        fifth(result)
    return result

async def coroutine_terminal_break(result, read, stop):
    while True:
        item = await read()
        if item is None:
            break
        consume(item)
        if stop(item):
            break
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def earlier_and_tail_break(first_flag, last_flag, result):
    while True:
        if first_flag:
            break
        step()
        if last_flag:
            break
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def protected_try_terminal(flag, result):
    try:
        while True:
            if flag:
                break
            step()
    except Exception:
        cleanup()
    first(result)
    second(result)
    third(result)
    fourth(result)
    fifth(result)
    return result

def protected_with_following(manager, flag, result):
    with manager:
        while True:
            if flag:
                break
            step()
        first(result)
        second(result)
        third(result)
        fourth(result)
        fifth(result)
    return result

