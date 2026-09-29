
assert ready, describe()
def check(value):
    assert value
    assert value > 0, f'bad value: {value}'
    return value
def only_assert():
    assert side_effect(), message()
class C:
    assert ready
    def method(self):
        assert self.ready
try:
    effect()
finally:
    assert ready
try:
    assert ready
except ValueError:
    assert recovery
try:
    assert ready
except:
    assert recovery
after = 1
try:
    assert ready
except:
    assert recovery
finally:
    after = 2
