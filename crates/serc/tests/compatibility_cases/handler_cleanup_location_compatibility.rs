use crate::common;

#[test]
fn matches_cpython_marshal_for_handler_cleanup_locations() {
    let source = r#"def uploader_shape(track, payload, count):
    if track.enabled:
        try:
            write(payload, track.endpoint)
            distribution("batch.cardinality", count)
        except UploadError:
            if not track.endpoint.startswith("/debugger/v1/diagnostics"):
                downgrade()
                write(payload, track.endpoint)
                distribution("batch.cardinality", count)
            else:
                raise
        except Exception:
            report(track)


def langchain_shape(manager, result):
    with manager:
        try:
            next(result)
        except Exception as error:
            if not isinstance(error, TimeoutError):
                assert False, f"Expected TimeoutError, got {error}"


def vision_shape():
    try:
        model = create_model()
        model = compile_model(model)
        value = model(input_value())
        print(f"compiled model output: {value.shape}")
    except RuntimeError:
        if platform == "win32":
            print("Successfully caught RuntimeError on win")
        else:
            raise


def dulwich_shape(path):
    try:
        unlink(path)
    except PermissionError:
        if platform == "win32":
            chmod(path, writable | readable)
            unlink(path)
        else:
            raise


def outside_with_control(result):
    try:
        next(result)
    except Exception as error:
        if not isinstance(error, TimeoutError):
            assert False, f"Expected TimeoutError, got {error}"


def explicit_else_control(manager, result):
    with manager:
        try:
            next(result)
        except Exception as error:
            if not isinstance(error, TimeoutError):
                assert False, f"Expected TimeoutError, got {error}"
            else:
                report(error)


def raise_instead_of_assert_control(manager, result):
    with manager:
        try:
            next(result)
        except Exception as error:
            if not isinstance(error, TimeoutError):
                raise AssertionError(error)
"#;
    common::assert_matches_cpython_marshal(source, "handler_cleanup_locations.py");
}

#[test]
fn matches_cpython_marshal_for_unnamed_terminating_and_handler_cleanup() {
    let source = r"def calendar_shape(expanded, value, previous):
    try:
        expanded.index(value)
    except ValueError:
        difference = nearest(value, expanded)
        if difference is not None and difference != 0:
            if previous:
                value = rewind(value, difference)
            else:
                value = advance(value, difference)
            return True, value
    return False, value


def inherited_return_shape(value, name, environment):
    if value:
        try:
            return construct(getattr(value, name), environment)
        except AttributeError:
            if is_name(value) and value.id == name:
                return value
    raise ValueError(name)


def inherited_raise_shape(value, labels, axis):
    if axis:
        try:
            return convert(value, labels)
        except KeyError:
            if is_scalar(value) and is_valid(value):
                if axis == 1:
                    return labels
                raise ValueError(value)
        except TypeError:
            recover(value)
        except ValueError:
            recover(labels)
    return labels


def three_term_and_shape(first, second, third, value):
    try:
        action()
    except Error:
        if first and second and third:
            return value
    after()


def false_and_value_control(value):
    try:
        action()
    except Error:
        if False and value:
            return value
    after()


def value_and_false_control(value):
    try:
        action()
    except Error:
        if value and False:
            return value
    after()


def simple_condition_control(first, value):
    try:
        action()
    except Error:
        if first:
            return value
    after()
";
    common::assert_matches_cpython_marshal(source, "unnamed_terminating_and_handler_cleanup.py");
}

#[test]
fn matches_cpython_cleanup_positions_for_folded_and_handlers() {
    let source = r"def true_and_value_control(value):
    try:
        action()
    except Error:
        if True and value:
            return value
    after()


def value_and_true_control(value):
    try:
        action()
    except Error:
        if value and True:
            return value
    after()


def false_and_true_control(value):
    try:
        action()
    except Error:
        if False and True:
            return value
    after()


def true_and_false_control(value):
    try:
        action()
    except Error:
        if True and False:
            return value
    after()


def true_and_true_control(value):
    try:
        action()
    except Error:
        if True and True:
            return value
    after()


def true_and_not_true_control(value):
    try:
        action()
    except Error:
        if True and not True:
            return value
    after()
";
    common::assert_matches_cpython_pop_except_positions(
        source,
        "folded_single_edge_and_handler_cleanup.py",
    );
}

#[test]
fn sole_handler_name_store_locates_manager_exit() {
    for (function, prefix, recovery) in [
        ("def", "", "recover(value)"),
        ("async def", "", "await recover(value)"),
        ("def", "    yield value\n", "recover(value)"),
        ("async def", "    yield value\n", "await recover(value)"),
    ] {
        for binding in ["", " as error"] {
            for terminal in ["return", "raise Error(value)"] {
                let source = format!(
                    "{function} f(manager, value):\n{prefix}    with manager:\n        try:\n            work(value)\n            {terminal}\n        except Error{binding}:\n            if invalid(value):\n                raise\n            result = {recovery}\n    consume(result)\n"
                );
                common::assert_matches_cpython_marshal(&source, "handler_name_manager_exit.py");
            }
        }
    }
}

#[test]
fn handler_store_location_preserves_continuation_barriers() {
    for assignment in [
        "first = result = recover(value)",
        "first, result = recover(value)",
        "result: object = recover(value)",
        "value.result = recover(value)",
        "value[0] = recover(value)",
    ] {
        let source = format!(
            "def f(manager, value):\n    with manager:\n        try:\n            return\n        except Error:\n            {assignment}\n        consume(value)\n    after()\n"
        );
        common::assert_matches_cpython_marshal(&source, "handler_store_continuation.py");
    }

    let source = r"def shared_handlers(manager, value):
    with manager:
        try:
            return
        except FirstError:
            result = recover(value)
        except SecondError:
            result = fallback(value)
    consume(result)


def shared_normal_and_handler(manager, value):
    with manager:
        try:
            result = work(value)
        except Error:
            result = recover(value)
    consume(result)


def compound_handler(manager, value, flag):
    with manager:
        try:
            return
        except Error:
            if flag:
                result = recover(value)
            else:
                result = fallback(value)
    consume(result)


def finalizer_barrier(manager, value):
    with manager:
        try:
            return
        except Error:
            result = recover(value)
        finally:
            cleanup(value)
    consume(result)
";
    common::assert_matches_cpython_marshal(source, "handler_store_source_barriers.py");
}
