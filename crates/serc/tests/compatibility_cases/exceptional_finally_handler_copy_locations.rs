use crate::common;

fn enclose(source: &str, manager: &str, finalizer: bool) -> String {
    let body = source.replace('\n', "\n    ");
    if finalizer {
        format!("try:\n    {body}\nfinally:\n    finish()")
    } else {
        format!("{manager}\n    {body}")
    }
}

#[test]
fn matches_cpython_exceptional_finally_handler_copy_locations() {
    for (function, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield flag\n"),
        ("async def", "    yield flag\n"),
    ] {
        let manager = if function == "async def" {
            "async with manager:"
        } else {
            "with manager:"
        };
        for name in ["", " as error"] {
            for finalizer in [false, true] {
                for (nested, condition, following) in [
                    (false, "flag", ""),
                    (false, "True", ""),
                    (true, "flag", ""),
                    (false, "flag", "\nvalue = other"),
                    (false, "flag", "\nafter()"),
                ] {
                    for body in ["value = other", "work()"] {
                        let handler = format!(
                            "try:\n    work()\nexcept Exception{name}:\n    if {condition}:\n        {body}{following}"
                        );
                        let cleanup = if nested {
                            enclose(&handler, manager, finalizer)
                        } else {
                            handler
                        };
                        let protected = format!(
                            "try:\n    work()\nfinally:\n    {}",
                            cleanup.replace('\n', "\n    ")
                        );
                        let source = format!(
                            "{function} f(flag, ready, value, other, manager):\n{suspension}    {}\n",
                            enclose(&protected, manager, finalizer).replace('\n', "\n    ")
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "exceptional_finally_handler_copy_locations.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_handler_finally_return_cleanup_locations() {
    for (filename, source) in [
        (
            "handler_finally_attribute_assignment_return_location_target.py",
            r"def f(load, manager, state, saved):
    try:
        return load()
    except ValueError:
        try:
            with manager():
                return load()
        finally:
            state.value = saved
",
        ),
        (
            "handler_finally_local_delete_return_location_target.py",
            r"def f(load, make_frame):
    try:
        return load()
    except NameError:
        frame = make_frame()
        try:
            return load(frame)
        except Exception:
            return {}
        finally:
            del frame
",
        ),
        (
            "handler_finally_call_return_location_control.py",
            r"def f(load, manager, cleanup):
    try:
        return load()
    except ValueError:
        try:
            with manager():
                return load()
        finally:
            cleanup()
",
        ),
        (
            "finally_attribute_assignment_no_outer_handler_control.py",
            r"def f(load, manager, state, saved):
    try:
        with manager():
            return load()
    finally:
        state.value = saved
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_original_jump_slot_before_attribute_store_reraise() {
    for value in ["0", "False", "None"] {
        let source = format!(
            "def f(self, flag, other):\n    try:\n        work()\n    finally:\n        if flag:\n            if other:\n                try:\n                    recover()\n                except Error:\n                    pass\n            self.value = {value}\n        else:\n            cleanup()\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "attribute_store_reraise_original_jump_slot.py",
        );
    }
}
