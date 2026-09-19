use crate::common;

#[test]
fn extra_not_taken_owner_nested_loop_target() {
    common::assert_matches_cpython_marshal(
        r#"class Worker:
    def run(self):
        print(f"worker started; name={self._config.name}; store={self._config.store}; directory={self._directory}", file=sink.stderr)
        try:
            while True:
                try:
                    self.step()
                    if self._state.requested() and (not self._state.pending()):
                        print("worker drained", file=sink.stderr)
                        return 0
                except Exception as error:
                    print(f"worker step failed: {error!r}", file=sink.stderr)
                self._sleep(self._config.interval)
        except KeyboardInterrupt:
            return 0
"#,
        "extra_not_taken_owner_nested_loop_target.py",
    );
}

#[test]
fn extra_not_taken_owner_single_handler_control() {
    common::assert_matches_cpython_marshal(
        r#"class Worker:
    def run(self):
        print(f"worker started; name={self._config.name}; store={self._config.store}; directory={self._directory}", file=sink.stderr)
        while True:
            try:
                self.step()
                if self._state.requested() and (not self._state.pending()):
                    print("worker drained", file=sink.stderr)
                    return 0
            except Exception as error:
                print(f"worker step failed: {error!r}", file=sink.stderr)
            self._sleep(self._config.interval)
"#,
        "extra_not_taken_owner_single_handler_control.py",
    );
}
