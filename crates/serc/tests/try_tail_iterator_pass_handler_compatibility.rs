#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_try_tail_iterator_pass_handler() {
    let source = r#"def minimal(shell, children, finalize):
    try:
        if shell.poll() is None:
            shell.kill()
            for child in children:
                try:
                    child.kill()
                except Missing.NoSuchProcess:
                    pass
    except (LookupError, ProcessLookupError):
        pass
    else:
        finalize(shell)


def preceded(shell, make_children, finalize):
    try:
        children = make_children(shell)
        if shell.poll() is None:
            shell.kill()
            for child in children:
                try:
                    child.kill()
                except LookupError:
                    pass
    except (LookupError, ProcessLookupError):
        pass
    else:
        finalize(shell)


def long_else(shell, make_children, finalize):
    try:
        children = make_children(shell)
        if shell.poll() is None:
            shell.kill()
            for child in children:
                try:
                    child.kill()
                except LookupError:
                    pass
    except (LookupError, ProcessLookupError):
        pass
    else:
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)
        finalize(shell)


def outer_without_else(shell, children):
    try:
        if shell.poll() is None:
            shell.kill()
            for child in children:
                try:
                    child.kill()
                except LookupError:
                    pass
    except (LookupError, ProcessLookupError):
        pass


def unconditional_loop(shell, children, finalize):
    try:
        shell.kill()
        for child in children:
            try:
                child.kill()
            except LookupError:
                pass
    except (LookupError, ProcessLookupError):
        pass
    else:
        finalize(shell)


def conditional_loop_without_inner_try(shell, children, finalize):
    try:
        if shell.poll() is None:
            shell.kill()
            for child in children:
                child.kill()
    except (LookupError, ProcessLookupError):
        pass
    else:
        finalize(shell)


def preceded_condition(shell, make_children, finalize, platform):
    try:
        if platform != "win32":
            shell.stop()
        children = make_children(shell)
        if shell.poll() is None:
            shell.kill()
            for child in children:
                try:
                    child.kill()
                except LookupError:
                    pass
    except (LookupError, ProcessLookupError):
        pass
    else:
        finalize(shell)


def actual_shape(shell, make_children, finalize, platform, quitting, queue, handler):
    timeout = shell.timeout
    if timeout is not None:
        shell.cancel(timeout)
        shell.timeout = None
    if shell.process is None:
        return
    try:
        if platform != "win32":
            shell.send_stop()
        children = make_children(shell.process).children()
        if shell.process.poll() is None:
            shell.process.kill()
            for child in children:
                try:
                    child.kill()
                except LookupError:
                    pass
    except (LookupError, ProcessLookupError):
        pass
    else:
        if not quitting:
            assert shell.thread is not None
            assert shell.thread.is_alive()
            if platform == "win32":
                status = 1
            else:
                status = -9
            while True:
                kind, text = queue.get(block=True)
                if (kind, text) != ("error", f"failed with status {status}."):
                    handler(kind, text)
                if kind == "end":
                    break
            shell.thread.join()
            handler("error", "Killed.")
            finalize(shell)
            finalize(shell)
            finalize(shell)
            finalize(shell)
            finalize(shell)
            finalize(shell)
            finalize(shell)
            finalize(shell)
            finalize(shell)
            finalize(shell)
            finalize(shell)
            finalize(shell)
            finalize(shell)
            finalize(shell)
            finalize(shell)
            finalize(shell)
    if not quitting:
        finalize(shell)
"#;
    common::assert_matches_cpython_marshal(source, "try_tail_iterator_pass_handler.py");
}
