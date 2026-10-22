def update_records(enabled, selected, records):
    if enabled:
        if selected:
            for record in records:
                record["key"] = selected
        assert checked(records)


def check_bound(disabled, expected, low, value, high):
    if not disabled and value == expected and low < value < high:
        assert valid(value)
    else:
        assert invalid(value)


def validate_result(enabled):
    try:
        result = work()
    except RuntimeError as error:
        record(error)
    else:
        if enabled:
            assert accepted(result)
        assert checked(result)

    assert finished()
