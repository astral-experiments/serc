def consecutive(manager, first, second):
    with manager:
        assert first is None
        assert second is not None


def short_circuit(manager, left, right):
    with manager:
        assert left is None or right is not None


def direct_control(manager, value):
    with manager:
        assert value is None


def if_control(manager, value):
    with manager:
        if value is None:
            action()
