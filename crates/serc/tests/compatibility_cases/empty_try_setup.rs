use crate::common;

#[test]
fn empty_try_keeps_original_setup_and_bare_handler() {
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        try: operation()
        except NameError: pass
        except ZeroDivisionError: pass
        except TypeError: pass
        try: pass
        except: pass
        try: pass
        finally: pass
        consume(item)
",
        "empty_try_setup.py",
    );
}
