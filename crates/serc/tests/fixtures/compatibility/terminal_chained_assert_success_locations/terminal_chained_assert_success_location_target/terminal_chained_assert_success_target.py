def check_encoding(size, data):
    value = int(data)
    assert codec.api.encode(value) == data
    if 0 < size < 20:
        assert codec.api.encode(-value) == b'-' + data


def check_decoding(size, data):
    value = int(data)
    assert codec.api.decode(data) == value
    if 0 < size < 20:
        assert codec.api.decode(b'-' + data) == -value
