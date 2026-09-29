def direct(flag):
    try:
        while flag:
            step()
    except Error:
        recover()
    outside()


def direct_not(ready):
    try:
        while not ready:
            step()
    except Error:
        recover()
    outside()


async def direct_async(flag):
    try:
        while flag:
            await step()
    except Error:
        recover()
    outside()


def followed_in_try(flag):
    try:
        while flag:
            step()
        inside()
    except Error:
        recover()
    outside()


def multiple_exits(first, second):
    try:
        while first and second:
            step()
    except Error:
        recover()
    outside()


def loop_else(flag):
    try:
        while flag:
            step()
        else:
            fallback()
    except Error:
        recover()
    outside()


def try_else(flag):
    try:
        while flag:
            step()
    except Error:
        recover()
    else:
        success()
    outside()


def raising_body(flag):
    try:
        while flag:
            raise Error
    except Error:
        recover()
    outside()


def followed_raising_body(flag):
    try:
        while flag:
            raise Error
        inside()
    except Error:
        recover()
    outside()


def finally_cleanup(flag):
    try:
        while flag:
            step()
    finally:
        cleanup()
    outside()


def terminal(flag):
    try:
        while flag:
            step()
    except Error:
        recover()
