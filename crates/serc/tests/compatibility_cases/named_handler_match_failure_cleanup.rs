use crate::common;

#[test]
fn matches_cpython_marshal_for_named_handler_final_pattern_cleanup() {
    let source = r"class Runner:
    async def process(self, items):
        skipped = 0
        for item in items:
            try:
                consume(item)
            except Exception as exc:
                match self.behavior:
                    case Behavior.SKIP:
                        skipped += 1
                        continue
                    case Behavior.RAISE:
                        raise exc
                    case Behavior.RETRY:
                        raise RuntimeError(
                            f'Cannot retry {exc} with behavior {self.behavior}'
                        ) from exc
        return skipped
";
    common::assert_matches_cpython_marshal(source, "named_handler_final_pattern_cleanup.py");
}

#[test]
fn matches_cpython_marshal_for_named_handler_wildcard_cleanup_control() {
    let source = r"class Runner:
    async def process(self, items):
        skipped = 0
        for item in items:
            try:
                consume(item)
            except Exception as exc:
                match self.behavior:
                    case Behavior.SKIP:
                        skipped += 1
                        continue
                    case Behavior.RAISE:
                        raise exc
                    case _:
                        raise RuntimeError(
                            f'Cannot retry {exc} with behavior {self.behavior}'
                        ) from exc
        return skipped
";
    common::assert_matches_cpython_marshal(source, "named_handler_wildcard_cleanup_control.py");
}

#[test]
fn matches_cpython_marshal_for_named_handler_match_success_cleanup_control() {
    let source = r"class Runner:
    async def process(self, items):
        skipped = 0
        for item in items:
            try:
                consume(item)
            except Exception as exc:
                match self.behavior:
                    case Behavior.SKIP:
                        skipped += 1
                        continue
                    case Behavior.RAISE:
                        raise exc
                    case Behavior.RETRY:
                        recover(exc)
        return skipped
";
    common::assert_matches_cpython_marshal(
        source,
        "named_handler_match_success_cleanup_control.py",
    );
}
