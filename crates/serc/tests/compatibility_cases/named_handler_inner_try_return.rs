use crate::common;

fn source(binding: &str) -> String {
    format!(
        r"async def collect(items):
    for value in items:
        try:
            try:
                await fetch(value)
            except Exception{binding}:
                try:
                    return value
                except Exception as fallback:
                    report(fallback)
                continue
        except Exception:
            recover()
    return []
"
    )
}

#[test]
fn named_handler_return_through_inner_try() {
    common::assert_matches_cpython_marshal(
        &source(" as error"),
        "named_handler_return_through_inner_try.py",
    );
}

#[test]
fn unnamed_handler_return_through_inner_try_control() {
    common::assert_matches_cpython_marshal(
        &source(""),
        "unnamed_handler_return_through_inner_try_control.py",
    );
}
