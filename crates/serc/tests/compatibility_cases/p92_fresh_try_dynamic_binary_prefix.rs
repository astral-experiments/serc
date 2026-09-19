use crate::common::assert_matches_cpython_marshal;

fn source(subscripts: bool) -> String {
    let (join_open, join_close, upper_bound) = if subscripts {
        ("[(", ")]", "count[1]")
    } else {
        ("+ (", ")", "count - 1")
    };
    r#"try:
    records = read_records(path)
    if isinstance(records, list):
        count = len(indices)
        if count == 0:
            sink.note('empty')
            sink.stop()

        key = f"slot::{path}::{'filtered' if filtered else 'all'}"
        if key not in sink.state:
            sink.state[key] = 0
        sink.state[key] = max(0, min(sink.state[key], count - 1))

        sink.note(
            f"Count: {len(records)} | Visible: {count} "
            __JOIN_OPEN__f"| mean: {mean:.3f}" if mean is not None else "| mean: unavailable"__JOIN_CLOSE__
        )
        sink.select(
            'Index',
            minimum=0,
            maximum=__UPPER_BOUND__,
            value=sink.state[key],
            step=1,
            key=key,
        )
        index = indices[sink.state[key]]
        record = records[index]
        try:
            if isinstance(record, dict):
                display = dict(record)
            else:
                display = record
        except Exception:
            display = record

        if isinstance(display, dict):
            display.setdefault('mean', mean)
        sink.show(display, expanded=expanded)
    elif isinstance(records, dict):
        sink.show(records, expanded=expanded)
    elif isinstance(records, str):
        sink.code(records)
    else:
        sink.write(records)
except Exception as error:
    sink.error(f"Unable to read {path}: {error}")
"#
    .replace("__JOIN_OPEN__", join_open)
    .replace("__JOIN_CLOSE__", join_close)
    .replace("__UPPER_BOUND__", upper_bound)
}

#[test]
fn matches_cpython_fresh_try_dynamic_binary_prefix() {
    assert_matches_cpython_marshal(&source(false), "fresh_try_dynamic_binary_prefix.py");
}

#[test]
fn matches_cpython_fresh_try_subscript_prefix_control() {
    assert_matches_cpython_marshal(&source(true), "fresh_try_subscript_prefix_control.py");
}
