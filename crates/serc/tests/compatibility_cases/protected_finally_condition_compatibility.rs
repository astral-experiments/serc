use crate::common;

#[test]
fn matches_cpython_marshal_for_protected_finally_condition_owners() {
    let source = r"def assert_compare_cleanup(manager, proc):
    with manager:
        try:
            proc.wait()
            assert proc.returncode == 1
        finally:
            if proc.poll() is None:
                proc.terminate()
                proc.wait()
    return proc

def attribute_cleanup(manager, value):
    with manager:
        prepare(value)
        try:
            result = load(value)
        finally:
            if value.locked:
                unlock(value)
    return result

def name_cleanup_control(manager, value, locked):
    with manager:
        try:
            result = load(value)
        finally:
            if locked:
                unlock(value)
    return result
";
    common::assert_matches_cpython_marshal(source, "protected_finally_condition_owners.py");
}

#[test]
fn matches_cpython_unlocated_manager_finally_join() {
    for (filename, source) in [
        (
            "manager_compound_guard_raise_cleanup_target.py",
            r"def store_checked(manager, operation):
    result = None
    with manager():
        try:
            result = operation()
        finally:
            if result is not None and not result.accepted:
                raise RuntimeError('rejected')
    return result
",
        ),
        (
            "manager_single_guard_raise_cleanup_control.py",
            r"def store_checked(manager, operation, failed):
    result = None
    with manager():
        try:
            result = operation()
        finally:
            if failed:
                raise RuntimeError('rejected')
    return result
",
        ),
        (
            "manager_generator_linear_cleanup_control.py",
            r"def managed_value(manager, cleanup):
    with manager():
        try:
            yield
        finally:
            cleanup()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_certified_generator_manager_finally_joins() {
    for (filename, source) in [
        (
            "manager_generator_guarded_while_cleanup_target.py",
            r"def managed_chunks(manager, finish, ready, timed_out, pause):
    with manager():
        try:
            yield
        finally:
            finish()
            if ready is not None:
                while not ready():
                    if timed_out():
                        break
                    pause()
",
        ),
        (
            "manager_generator_elif_cleanup_target.py",
            r"def managed_value(manager, primary, secondary, restore, remove):
    with manager():
        try:
            yield
        finally:
            if primary():
                restore()
            elif secondary():
                remove()
",
        ),
        (
            "manager_generator_elif_cleanup_else_control.py",
            r"def managed_value(manager, primary, secondary, restore, remove, fallback):
    with manager():
        try:
            yield
        finally:
            if primary():
                restore()
            elif secondary():
                remove()
            else:
                fallback()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_coroutine_manager_elif_cleanup_owner() {
    let source = r"async def managed_value(manager, operation, primary, secondary, restore, remove):
    with manager():
        try:
            await operation()
        finally:
            if primary():
                restore()
            elif secondary():
                remove()
";
    common::assert_matches_cpython_marshal(
        source,
        "manager_coroutine_elif_cleanup_owner_control.py",
    );
}

#[test]
fn matches_cpython_generator_manager_handler_join_owner() {
    for (filename, source) in [
        (
            "manager_generator_elif_cleanup_handler_control.py",
            r"def managed_value(manager, primary, secondary, restore, remove, recover):
    with manager():
        try:
            yield
        except ValueError:
            recover()
        finally:
            if primary():
                restore()
            elif secondary():
                remove()
",
        ),
        (
            "manager_generator_elif_cleanup_named_handler_boundary.py",
            r"def managed_value(manager, primary, secondary, restore, remove, recover):
    with manager():
        try:
            yield
        except ValueError as error:
            recover(error)
        finally:
            if primary():
                restore()
            elif secondary():
                remove()
",
        ),
        (
            "manager_generator_elif_cleanup_branch_handler_boundary.py",
            r"def managed_value(manager, primary, secondary, restore, remove, recover):
    with manager():
        try:
            yield
        except ValueError:
            if primary():
                recover()
        finally:
            if primary():
                restore()
            elif secondary():
                remove()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
