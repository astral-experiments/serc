def check_gradients(values):
    for left, right in values:
        if left is None:
            assert right is None and (right == 0).all(), right
        else:
            assert left.ready is None
            assert right.ready is None
            assert (left == right).all(), (left, right)
