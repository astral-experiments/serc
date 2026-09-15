use crate::common;

#[test]
fn matches_cpython_marshal_for_lexical_with_finally_return_unwind() {
    let source = r"def manager_inside_finally(manager, cleanup):
    try:
        with manager as value:
            return value.read()
    finally:
        cleanup(flag=True)

def multiple_managers_inside_finally(manager, other, cleanup, value):
    try:
        with manager, other:
            return value.read()
    finally:
        cleanup(flag=True)

def finally_inside_manager(manager, cleanup, value):
    with manager:
        try:
            return value.read()
        finally:
            cleanup(flag=True)

def finally_inside_iterator(manager, cleanup, items):
    for value in items:
        try:
            with manager:
                return value.read()
        finally:
            cleanup(flag=True)

def handler_inside_manager(manager):
    with manager:
        try:
            return call()
        except Error as error:
            raise convert(error) from error
";
    common::assert_matches_cpython_marshal(source, "with_finally_return_unwind.py");
}

#[test]
fn awaited_manager_return_none_cleanup_target() {
    common::assert_matches_cpython_marshal(
        &awaited_manager_return_none_cleanup_source("            return result"),
        "awaited_manager_return_none_cleanup_target.py",
    );
}

#[test]
fn awaited_manager_return_none_cleanup_after_with_control() {
    common::assert_matches_cpython_marshal(
        &awaited_manager_return_none_cleanup_source("        return result"),
        "awaited_manager_return_none_cleanup_after_with_control.py",
    );
}

fn awaited_manager_return_none_cleanup_source(return_line: &str) -> String {
    format!(
        r#"async def publish(payload, state):
    temporary = None
    try:
        with make_directory(prefix="stage.") as directory:
            temporary = make_path(directory)
            result = build(payload)
            await send_path(temporary, state, overwrite=True)
            await send_record(
                state,
                status(
                    phase="complete",
                    identity=result["id"],
                ),
                overwrite=True,
            )
{return_line}
    except Exception as error:
        await send_record(
            state,
            status(
                phase="failed",
                error={{"message": str(error), "retryable": True}},
            ),
            overwrite=True,
        )
        raise
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
"#
    )
}
