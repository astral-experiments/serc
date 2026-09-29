
def check(items):
    for item in items:
        if item.selected:
            assert item.done or False
        else:
            raise ValueError(item)
    finish()
