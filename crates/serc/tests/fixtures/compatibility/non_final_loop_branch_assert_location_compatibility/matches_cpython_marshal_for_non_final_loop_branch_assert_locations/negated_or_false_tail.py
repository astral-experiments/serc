
def check(items):
    for item in items:
        if item.selected:
            assert not (item.done or False)
        else:
            raise ValueError(item)
    finish()
