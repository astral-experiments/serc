def action(value):
    return value


def expression_arms(flag):
    try:
        action("try")
    finally:
        if flag:
            action("left")
        else:
            action("right")


def first_pass(flag):
    try:
        action("try")
    finally:
        if flag:
            pass
        else:
            action("right")


def else_pass(flag):
    try:
        action("try")
    finally:
        if flag:
            action("left")
        else:
            pass


def both_pass(flag):
    try:
        action("try")
    finally:
        if flag:
            pass
        else:
            pass


def no_else(flag):
    try:
        action("try")
    finally:
        if flag:
            action("left")


def first_arm_terminates(flag):
    try:
        action("try")
    finally:
        if flag:
            return action("left")
        else:
            action("right")


def both_arms_terminate(flag):
    try:
        action("try")
    finally:
        if flag:
            return action("left")
        else:
            return action("right")


def trailing_statement(flag):
    try:
        action("try")
    finally:
        if flag:
            action("left")
        else:
            action("right")
        action("trailing")


def generator(flag):
    try:
        yield action("try")
    finally:
        if flag:
            action("left")
        else:
            action("right")


async def coroutine(flag):
    try:
        action("try")
    finally:
        if flag:
            action("left")
        else:
            action("right")
