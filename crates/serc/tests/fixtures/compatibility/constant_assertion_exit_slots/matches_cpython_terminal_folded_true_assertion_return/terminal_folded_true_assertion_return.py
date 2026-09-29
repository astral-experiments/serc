def check_value(extra, value):
    assert extra(value) if extra else True
