use crate::common;

#[test]
fn terminal_chained_assert_success_location_target() {
    let source = r"def check_encoding(size, data):
    value = int(data)
    assert codec.api.encode(value) == data
    if 0 < size < 20:
        assert codec.api.encode(-value) == b'-' + data


def check_decoding(size, data):
    value = int(data)
    assert codec.api.decode(data) == value
    if 0 < size < 20:
        assert codec.api.decode(b'-' + data) == -value
";
    common::assert_matches_cpython_marshal(source, "terminal_chained_assert_success_target.py");
}

#[test]
fn terminal_chained_assert_success_location_control() {
    let source = r"def check_encoding(size, data):
    value = int(data)
    assert codec.api.encode(value) == data
    if size < 20:
        assert codec.api.encode(-value) == b'-' + data


def check_decoding(size, data):
    value = int(data)
    assert codec.api.decode(data) == value
    if size < 20:
        assert codec.api.decode(b'-' + data) == -value
";
    common::assert_matches_cpython_marshal(source, "terminal_chained_assert_success_control.py");
}
