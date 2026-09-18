use crate::common;

#[test]
fn conditional_nested_recovery_else_borrow_matches_cpython() {
    let cases = [
        (
            "conditional_nested_recovery_else_target.py",
            r"async def collect(fetch, recover, prefer, fallback, report):
    try:
        values = await fetch()
        if prefer and fallback:
            prior = values
            try:
                extras = await recover()
            except Exception:
                if not prior:
                    raise
                report()
                values = prior
            else:
                values = [*prior, *extras]
    except Exception:
        report()
        return {}

    grouped = {}
    for value in values:
        grouped.setdefault(value.key, []).append(value)
    return grouped
",
        ),
        (
            "conditional_nested_recovery_else_control.py",
            r"async def collect(fetch, recover, prefer, fallback, report):
    try:
        values = await fetch()
        if prefer:
            prior = values
            try:
                extras = await recover()
            except Exception:
                if not prior:
                    raise
                report()
                values = prior
            else:
                values = [*prior, *extras]
    except Exception:
        report()
        return {}

    grouped = {}
    for value in values:
        grouped.setdefault(value.key, []).append(value)
    return grouped
",
        ),
    ];
    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
