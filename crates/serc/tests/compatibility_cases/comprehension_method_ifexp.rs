use crate::common;

#[test]
fn async_generator_comprehension_method_ifexp() {
    common::assert_matches_cpython_marshal(
        r#"async def sample(rows, ready, sink):
    try:
        yield
    finally:
        sink([f"{key}:{value}" for key, value in rows].append(
            f"flag:{'yes' if ready else 'no'}"
        ))
"#,
        "async_generator_comprehension_method_ifexp.py",
    );
}

#[test]
fn generator_comprehension_method_ifexp() {
    common::assert_matches_cpython_marshal(
        r#"def sample(rows, ready, sink):
    try:
        yield
    finally:
        sink([f"{key}:{value}" for key, value in rows].append(
            f"flag:{'yes' if ready else 'no'}"
        ))
"#,
        "generator_comprehension_method_ifexp.py",
    );
}

#[test]
fn async_generator_comprehension_call_control() {
    common::assert_matches_cpython_marshal(
        r#"async def sample(rows, ready, sink):
    try:
        yield
    finally:
        sink(f"{sink([f'{key}:{value}' for key, value in rows])}{'yes' if ready else 'no'}")
"#,
        "async_generator_comprehension_call_control.py",
    );
}

#[test]
fn async_generator_plain_method_ifexp_control() {
    common::assert_matches_cpython_marshal(
        r#"async def sample(rows, ready, sink):
    try:
        yield
    finally:
        sink(rows.append(
            f"flag:{'yes' if ready else 'no'}"
        ))
"#,
        "async_generator_plain_method_ifexp_control.py",
    );
}
