def verify(manager, value, expected):
    with manager():
        assert (
            value == expected
            if True
            else False
        )
