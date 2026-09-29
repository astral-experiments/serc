def factory_1(first, second, third, returned, raised, cause, choose):
    def inner():
        try:
            yield first
            return returned
        finally:
            raise raised
    return inner

def factory_2(first, second, third, returned, raised, cause, choose):
    def inner():
        try:
            yield first
            yield second
            yield third
            return returned
        finally:
            raise raised
    return inner

def factory_3(first, second, third, returned, raised, cause, choose):
    def inner():
        try:
            yield first
            return 42
        finally:
            raise raised
    return inner

def factory_4(first, second, third, returned, raised, cause, choose):
    def inner():
        try:
            yield first
            return returned()
        finally:
            raise raised()
    return inner

def factory_5(first, second, third, returned, raised, cause, choose):
    def inner():
        try:
            yield first
            return returned
        finally:
            raise raised from cause
    return inner

def inner_6(first, returned, raised):
    try:
        yield first
        return returned
    finally:
        raise raised
