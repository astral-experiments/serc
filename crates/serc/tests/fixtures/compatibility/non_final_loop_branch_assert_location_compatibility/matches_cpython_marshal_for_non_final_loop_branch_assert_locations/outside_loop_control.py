
def check(item):
    if item.selected:
        assert not item.done
    else:
        raise ValueError(item)
