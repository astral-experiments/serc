
debug = __debug__
constants = (__debug__, not __debug__, (__debug__,))
choice = 'debug' if __debug__ else 'optimized'
if __debug__:
    branch = 1
else:
    branch = 2
def function(first):
    while __debug__:
        break
    if not __debug__:
        first += 1
    return __debug__
def conjunction(value):
    return __debug__ and value
def disjunction(value):
    return __debug__ or value
class C:
    debug = __debug__
    def method(self, default=__debug__):
        return lambda: (__debug__, default)
values = [__debug__ for item in items if __debug__ or item]
generator = (__debug__ for item in items if not __debug__)
