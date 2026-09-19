use crate::common;

#[test]
fn matches_cpython_handler_fed_finally_boundaries() {
    for body in [
        "raise Error",
        "return value",
        "while True:\n                action()",
    ] {
        for finalbody in [
            "finish()",
            "result = value",
            "finish(value if value else None)\n            result = value",
        ] {
            let source = format!(
                "def f(manager, value):\n    with manager:\n        try:\n            {body}\n        except Error:\n            recover()\n        finally:\n            {finalbody}\n"
            );
            common::assert_matches_cpython_marshal(&source, "handler_fed_finally_boundary.py");
        }
    }
}

#[test]
fn matches_cpython_finally_boundaries_in_multiple_managers() {
    let source = r"def multiple_managers(first, second, third, value):
    with (
        first,
        second,
        third,
    ):
        try:
            return value
        except Error:
            recover()
        finally:
            finish()

def conditional_finalizer(manager, value):
    with manager:
        try:
            raise Error
        except Error:
            recover()
        finally:
            if value:
                finish()

def terminating_handlers(manager, value):
    with manager:
        try:
            return value
        except Error:
            raise
        finally:
            finish()

def followed_finalizer(manager):
    with manager:
        try:
            while True:
                action()
        except Error:
            recover()
        finally:
            finish()
        following()
";
    common::assert_matches_cpython_marshal(source, "multiple_manager_finally_boundaries.py");
}
