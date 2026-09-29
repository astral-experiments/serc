def verify(manager, value, expected):
    with manager():
        assert (
            value == expected
            or False
        )
