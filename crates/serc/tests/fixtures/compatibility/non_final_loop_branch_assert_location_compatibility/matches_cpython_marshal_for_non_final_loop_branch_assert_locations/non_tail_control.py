
def check(items):
    for item in items:
        if item.selected:
            assert not item.done
            consume(item)
        else:
            raise ValueError(item)
    finish()
