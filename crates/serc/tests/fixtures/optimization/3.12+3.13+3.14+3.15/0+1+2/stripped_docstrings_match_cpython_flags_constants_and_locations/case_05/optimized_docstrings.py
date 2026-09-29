"module docstring"
"a later string expression"
def function():
    "function docstring"
    "another string expression"
    return "value"
def empty():
    "only a docstring"
async def coroutine():
    "coroutine docstring"
    return 1
def generator():
    "generator docstring"
    yield 1
class C:
    "class docstring"
    def method(self):
        "method docstring"
        return __class__
    class Empty:
        "only a nested class docstring"
