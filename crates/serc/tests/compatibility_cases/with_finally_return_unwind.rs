use crate::common;

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
