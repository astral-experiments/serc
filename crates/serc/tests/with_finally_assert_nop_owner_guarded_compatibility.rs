#[allow(dead_code)]
mod common;

#[test]
fn guarded_with_finally_assert_nop_owner_positives_match_cpython_marshal() {
    let source = r#"def spack_followed_cleanup(manager, repo, old_path, pkg):
    with manager:
        try:
            repo.python_path = None
            assert github_url(pkg) is None
            repo.python_path = "/missing"
            assert github_url(pkg) is None
        finally:
            repo.python_path = old_path
        after()


async def coroutine(manager, condition):
    with manager:
        try:
            assert await check(condition)
        finally:
            cleanup()


async def asynchronous_with(manager, condition):
    async with manager:
        try:
            assert condition
        finally:
            await cleanup()


def generator(manager, condition):
    yield seed()
    with manager:
        try:
            assert condition
        finally:
            cleanup()


async def async_generator(manager, condition):
    yield seed()
    with manager:
        try:
            assert condition
        finally:
            await cleanup()
"#;

    common::assert_matches_cpython_marshal(source, "with_finally_assert_nop_owner_positives.py");
}

#[test]
fn guarded_with_finally_assert_nop_owner_exclusions_match_cpython_marshal() {
    let source = r"def no_with(condition):
    try:
        assert condition
    finally:
        cleanup()


def non_assert_tail(manager, condition):
    with manager:
        try:
            consume(condition)
        finally:
            cleanup()


def handler_form(manager, condition):
    with manager:
        try:
            assert condition
        except Error:
            pass
        finally:
            cleanup()
        after()


def enclosing_try(manager, condition):
    with manager:
        try:
            try:
                assert condition
            finally:
                cleanup()
        except Error:
            pass


def enclosing_handler(manager, condition):
    with manager:
        try:
            body()
        except Error as error:
            try:
                assert condition
            finally:
                consume(error)


def enclosing_finally(manager, condition):
    with manager:
        try:
            raise Error()
        finally:
            try:
                assert condition
            finally:
                cleanup()
";

    common::assert_matches_cpython_marshal(source, "with_finally_assert_nop_owner_exclusions.py");
}
