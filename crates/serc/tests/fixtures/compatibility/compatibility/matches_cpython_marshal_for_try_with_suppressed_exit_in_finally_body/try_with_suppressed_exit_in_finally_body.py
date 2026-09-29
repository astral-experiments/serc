def terminal_nested(manager):
    try:
        try:
            with manager:
                action()
        except Error:
            recover()
    finally:
        cleanup()


def following_nested(manager):
    try:
        try:
            with manager:
                action()
        except Error:
            recover()
        after()
    finally:
        cleanup()


def with_not_final(manager):
    try:
        try:
            with manager:
                action()
            after()
        except Error:
            recover()
    finally:
        cleanup()


def sole_finally(manager):
    try:
        with manager:
            action()
    finally:
        cleanup()
