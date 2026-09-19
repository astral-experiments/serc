use crate::common;

#[test]
fn folded_while_import_borrow_target() {
    common::assert_matches_cpython_marshal(
        r"class Worker:
    def run(self, seed):
        attempt = 0
        while True:
            attempt += 1
            token = f'{self.context}:{seed}:{attempt}'
            from support import deadline

            @deadline(10)
            def produce(token=token):
                return make_value(token, self.config)

            try:
                value = produce()
                if value is None:
                    continue
            except (TimeoutError, SystemError):
                continue
            return value
",
        "folded_while_import_borrow_target.py",
    );
}

#[test]
fn folded_while_import_borrow_runtime_control() {
    common::assert_matches_cpython_marshal(
        r"class Worker:
    def run(self, seed):
        attempt = 0
        while ready():
            attempt += 1
            token = f'{self.context}:{seed}:{attempt}'
            from support import deadline

            @deadline(10)
            def produce(token=token):
                return make_value(token, self.config)

            try:
                value = produce()
                if value is None:
                    continue
            except (TimeoutError, SystemError):
                continue
            return value
",
        "folded_while_import_borrow_runtime_control.py",
    );
}
