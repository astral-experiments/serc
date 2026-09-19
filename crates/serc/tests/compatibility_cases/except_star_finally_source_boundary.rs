use crate::common;

#[test]
fn except_star_manager_finally() {
    common::assert_matches_cpython_marshal(
        r#"async def run(self, manager, group):
    with manager:
        await self.ready()
        started = clock.now()
        try:
            async with group as tasks:
                for index in range(self.count):
                    tasks.create_task(self.work(index))
        except* GroupError:
            pass
        finally:
            elapsed = clock.now() - started
            message = f"runtime {elapsed:,.1f}; tokens {self.tokens}"
            if self.errors:
                message += " errors"
            for error in self.errors:
                message += f"{error}: {self.errors[error]}"
            logger.info(message)
    return 0
"#,
        "except_star_manager_finally.py",
    );
}

#[test]
fn except_star_manager_regular_except_control() {
    common::assert_matches_cpython_marshal(
        r#"async def run(self, manager, group):
    with manager:
        await self.ready()
        started = clock.now()
        try:
            async with group as tasks:
                for index in range(self.count):
                    tasks.create_task(self.work(index))
        except GroupError:
            pass
        finally:
            elapsed = clock.now() - started
            message = f"runtime {elapsed:,.1f}; tokens {self.tokens}"
            if self.errors:
                message += " errors"
            for error in self.errors:
                message += f"{error}: {self.errors[error]}"
            logger.info(message)
    return 0
"#,
        "except_star_manager_regular_except_control.py",
    );
}

#[test]
fn except_star_manager_no_cleanup_prefix_control() {
    common::assert_matches_cpython_marshal(
        r#"async def run(self, manager, group):
    with manager:
        await self.ready()
        started = clock.now()
        try:
            async with group as tasks:
                for index in range(self.count):
                    tasks.create_task(self.work(index))
        except* GroupError:
            pass
        finally:
            if self.errors:
                message += " errors"
            for error in self.errors:
                message += f"{error}: {self.errors[error]}"
            logger.info(message)
    return 0
"#,
        "except_star_manager_no_cleanup_prefix_control.py",
    );
}

#[test]
fn except_star_manager_no_finally_control() {
    common::assert_matches_cpython_marshal(
        r#"async def run(self, manager, group):
    with manager:
        await self.ready()
        try:
            async with group as tasks:
                for index in range(self.count):
                    tasks.create_task(self.work(index))
        except* GroupError:
            pass
        message = self.message
        if self.errors:
            message += " errors"
        logger.info(message)
    return 0
"#,
        "except_star_manager_no_finally_control.py",
    );
}

#[test]
fn matches_cpython_normalized_except_star_raise_loop() {
    for (filename, source) in [
        (
            "async_generator_named_star_linear_raise_control.py",
            r"async def target(manager, leaves):
    try:
        async with manager() as value:
            yield value
    except* Exception as group:
        leaf = leaves(group)
        raise leaf from group
",
        ),
        (
            "async_generator_named_star_runtime_raise_loop.py",
            r"async def target(manager, leaves, error_type):
    try:
        async with manager() as value:
            yield value
    except* Exception as group:
        for leaf in leaves(group):
            if isinstance(leaf, error_type):
                raise leaf from group
        raise
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
