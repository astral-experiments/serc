def pass_body(value, manager):
    try:
        with manager:
            pass
    except Error:
        pass
    consume(value)

def active_body(value, manager):
    try:
        with manager:
            action()
    except Error:
        pass
    consume(value)

def following_try(value, manager):
    try:
        with manager:
            action()
    except Error:
        pass
    try:
        consume(value)
    except OtherError:
        raise

def terminal_handler(value, manager):
    try:
        with manager:
            action()
    except Error:
        return
    consume(value)

def nested_try(value, manager):
    try:
        try:
            with manager:
                action()
        except InnerError:
            recover()
        consume(value)
    except OuterError:
        recover_outer()
    return value

def finally_guard(value, manager):
    try:
        with manager:
            action()
    finally:
        cleanup()
    try:
        consume(value)
    except Error:
        raise

def combined_loop(manager, items):
    for item in items:
        try:
            with manager:
                action(item)
        except Error:
            recover()
        finally:
            cleanup()

def combined_following(manager):
    try:
        with manager:
            action()
    except Error:
        recover()
    finally:
        cleanup()
    after()

def combined_terminal(manager):
    try:
        with manager:
            action()
    except Error:
        recover()
    finally:
        cleanup()

def ordinary_guard(value):
    try:
        action()
    except Error:
        pass
    try:
        consume(value)
    except OtherError:
        raise
