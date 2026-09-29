
def check(items):
    for item in items:
        if item.selected:
            assert item.done
        else:
            raise ValueError(item)
    finish()
