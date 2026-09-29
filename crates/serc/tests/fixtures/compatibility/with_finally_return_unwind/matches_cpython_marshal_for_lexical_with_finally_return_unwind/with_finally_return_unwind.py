def manager_inside_finally(manager, cleanup):
    try:
        with manager as value:
            return value.read()
    finally:
        cleanup(flag=True)

def multiple_managers_inside_finally(manager, other, cleanup, value):
    try:
        with manager, other:
            return value.read()
    finally:
        cleanup(flag=True)

def finally_inside_manager(manager, cleanup, value):
    with manager:
        try:
            return value.read()
        finally:
            cleanup(flag=True)

def finally_inside_iterator(manager, cleanup, items):
    for value in items:
        try:
            with manager:
                return value.read()
        finally:
            cleanup(flag=True)

def handler_inside_manager(manager):
    with manager:
        try:
            return call()
        except Error as error:
            raise convert(error) from error
