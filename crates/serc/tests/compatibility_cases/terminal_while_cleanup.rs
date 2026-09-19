use crate::common;

const RETRY_CLEANUP: &str = r#"
def retry(request):
    result = None
    resource = None
    command = "worker " + request.name
    try:
        resource = start(command, isolated=True, shell=True)
        attempts = 10
        while attempts > 0:
            try:
                result = client.complete(
                    name="dummy",
                    key="dummy",
                    endpoint="local",
                    prompt=request.prompt,
                    limit=10,
                    scores=0,
                    echo=True,
                    temperature=0.0,
                )
            except Exception as error:
                report(f"Retry pending: {error}")
                pause(10)
                attempts -= 1
            else:
                break
    except Exception as error:
        report("Worker failed.")
        raise error
    finally:
        if resource is not None:
            process.stop(process.group(resource.pid), signal.STOP)
    if result is None:
        raise RuntimeError("No result")
    return result
"#;

#[test]
fn matches_cpython_terminal_while_raising_handler_cleanup_target() {
    common::assert_matches_cpython_marshal(
        RETRY_CLEANUP,
        "terminal_while_raising_handler_cleanup_target.py",
    );
}

#[test]
fn matches_cpython_terminal_while_linear_handler_cleanup_control() {
    let source = RETRY_CLEANUP.replace("        raise error\n", "");
    common::assert_matches_cpython_marshal(
        &source,
        "terminal_while_linear_handler_cleanup_control.py",
    );
}
