use crate::common;

#[test]
fn matches_cpython_marshal_for_shared_if_else_finally_reraise() {
    let source = r#"def action(value):
    return value


def expression_arms(flag):
    try:
        action("try")
    finally:
        if flag:
            action("left")
        else:
            action("right")


def first_pass(flag):
    try:
        action("try")
    finally:
        if flag:
            pass
        else:
            action("right")


def else_pass(flag):
    try:
        action("try")
    finally:
        if flag:
            action("left")
        else:
            pass


def both_pass(flag):
    try:
        action("try")
    finally:
        if flag:
            pass
        else:
            pass


def no_else(flag):
    try:
        action("try")
    finally:
        if flag:
            action("left")


def first_arm_terminates(flag):
    try:
        action("try")
    finally:
        if flag:
            return action("left")
        else:
            action("right")


def both_arms_terminate(flag):
    try:
        action("try")
    finally:
        if flag:
            return action("left")
        else:
            return action("right")


def trailing_statement(flag):
    try:
        action("try")
    finally:
        if flag:
            action("left")
        else:
            action("right")
        action("trailing")


def generator(flag):
    try:
        yield action("try")
    finally:
        if flag:
            action("left")
        else:
            action("right")


async def coroutine(flag):
    try:
        action("try")
    finally:
        if flag:
            action("left")
        else:
            action("right")
"#;

    common::assert_matches_cpython_marshal(source, "exceptional_finally_shared_reraise.py");
}

#[test]
fn matches_cpython_manager_delete_reraise_copy_widths() {
    for (filename, source) in [
        (
            "exceptional_manager_four_delete_reraise_control.py",
            r"def run(lock, again):
    try:
        while again():
            first, second, third, fourth = values()
            try:
                work()
            finally:
                with lock:
                    touch()
                del first
                del second
                del third
                del fourth
    finally:
        finish()
",
        ),
        (
            "exceptional_manager_three_delete_reraise_limit.py",
            r"def run(lock, again):
    try:
        while again():
            first, second, third, fourth = values()
            try:
                work()
            finally:
                with lock:
                    touch()
                del first
                del second
                del third
    finally:
        finish()
",
        ),
        (
            "exceptional_manager_two_delete_reraise_target.py",
            r"def run(lock, again):
    try:
        while again():
            first, second, third, fourth = values()
            try:
                work()
            finally:
                with lock:
                    touch()
                del first
                del second
    finally:
        finish()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
