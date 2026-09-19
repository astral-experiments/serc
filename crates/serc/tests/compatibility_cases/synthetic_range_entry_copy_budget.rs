use crate::common;

#[test]
fn synthetic_range_entries_preserve_original_copy_budget() {
    for (filename, source) in [
        (
            "polling_chain_bare_return.py",
            r"def poll(url, timeout, interval):
    start = clock.time()
    last_error = None
    while clock.time() - start < timeout:
        try:
            response = client.get(url, timeout=0.5)
            if 200 <= response.status < 500:
                return
        except Exception as error:
            last_error = error
        clock.sleep(interval)
    raise TimeoutError(url) from last_error
",
        ),
        (
            "polling_chain_value_return.py",
            r"def poll(url, timeout, interval):
    start = clock.time()
    last_error = None
    while clock.time() - start < timeout:
        try:
            response = client.get(url, timeout=0.5)
            if 200 <= response.status < 500:
                return response
        except Exception as error:
            last_error = error
        clock.sleep(interval)
    raise TimeoutError(url) from last_error
",
        ),
        (
            "polling_chain_unnamed_handler.py",
            r"def poll(url, timeout, interval):
    start = clock.time()
    last_error = None
    while clock.time() - start < timeout:
        try:
            response = client.get(url, timeout=0.5)
            if 200 <= response.status < 500:
                return
        except Exception:
            last_error = None
        clock.sleep(interval)
    raise TimeoutError(url) from last_error
",
        ),
        (
            "nested_try_chain_bare_return.py",
            r"def poll(value, limit):
    while ready():
        try:
            try:
                if 0 <= value < limit:
                    return
            except ValueError:
                recover()
        except Exception:
            retry()
        advance()
",
        ),
        (
            "unprotected_chain_bare_return.py",
            r"def poll(value, limit):
    while ready():
        if 0 <= value < limit:
            return
        advance()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
