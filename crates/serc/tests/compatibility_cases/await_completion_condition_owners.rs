use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_nested_try_after_await_condition_owner() {
    let source = r"async def f(self, enabled):
    if enabled:
        while True:
            try:
                await connect(self.address)
                announce(self.address)
                await pause()
                try:
                    self.address = resolver.resolve(self.original)
                    if not self.local:
                        self.peer = self.address
                except Exception:
                    pass
                break
            except Exception:
                if self.original != 'local':
                    try:
                        self.address = resolver.resolve(self.original)
                        if not self.local:
                            self.peer = self.address
                    except Exception:
                        pass
                await pause()
    await self.start()
";
    assert_matches_cpython_marshal(source, "nested_try_after_await_condition_owner.py");
}

#[test]
fn matches_cpython_nested_try_before_await_condition_owner_control() {
    let source = r"async def f(self):
    try:
        try:
            self.address = resolver.resolve(self.original)
            if not self.local:
                self.peer = self.address
        except Exception:
            pass
        await pause()
    except Exception:
        recover()
";
    assert_matches_cpython_marshal(source, "nested_try_before_await_condition_owner_control.py");
}
