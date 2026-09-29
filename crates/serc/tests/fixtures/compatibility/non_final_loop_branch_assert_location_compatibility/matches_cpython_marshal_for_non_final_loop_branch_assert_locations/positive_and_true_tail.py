
def check(items):
    for item in items:
        if item.selected:
            assert item.done and True
        else:
            raise ValueError(item)
    finish()
