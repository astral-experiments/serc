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
