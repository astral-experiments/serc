
def check(items):
    for item in items:
        if item.selected:
            assert not item.done, "already done"
        else:
            raise ValueError(item)
    finish()
