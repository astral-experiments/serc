def check(active, value, user, values):
    if active:
        assert (
            value == 'max'
            or value == 'auto'
            or str(value) == '8'
            or str(value) == '0'
            or user == 'health'
            or (str(value) == '4' and any(item for item in values))
        ), 'unsupported'
