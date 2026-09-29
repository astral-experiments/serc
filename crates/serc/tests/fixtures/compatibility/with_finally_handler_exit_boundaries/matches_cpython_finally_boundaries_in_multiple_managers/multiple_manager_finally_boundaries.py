def multiple_managers(first, second, third, value):
    with (
        first,
        second,
        third,
    ):
        try:
            return value
        except Error:
            recover()
        finally:
            finish()

def conditional_finalizer(manager, value):
    with manager:
        try:
            raise Error
        except Error:
            recover()
        finally:
            if value:
                finish()

def terminating_handlers(manager, value):
    with manager:
        try:
            return value
        except Error:
            raise
        finally:
            finish()

def followed_finalizer(manager):
    with manager:
        try:
            while True:
                action()
        except Error:
            recover()
        finally:
            finish()
        following()
