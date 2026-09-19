use crate::common;

#[test]
fn matches_cpython_marshal_for_match_handler_call_fallthrough_join() {
    let source = r"from __future__ import annotations

class Runner:
    async def execute(self, work, *, policy, argument=None):
        if argument is not None:
            work = work.copy(update={'arg': argument})
        try:
            result = await self.run(work, policy=policy)
        except Exception:
            match policy:
                case Policy.STOP:
                    log.error('stop', exc_info=True)
                    raise
                case Policy.CONTINUE:
                    log.warning('continue', exc_info=True)
                    if enabled(PATCH):
                        self.record()
                    result = None
                case _:
                    unreachable(policy)
        else:
            self.record()
        return result
";
    common::assert_matches_cpython_marshal(source, "match_handler_call_join.py");
}

#[test]
fn matches_cpython_marshal_for_match_handler_raise_join_control() {
    let source = r"from __future__ import annotations

class Runner:
    async def execute(self, work, *, policy, argument=None):
        if argument is not None:
            work = work.copy(update={'arg': argument})
        try:
            result = await self.run(work, policy=policy)
        except Exception:
            match policy:
                case Policy.STOP:
                    log.error('stop', exc_info=True)
                    raise
                case Policy.CONTINUE:
                    log.warning('continue', exc_info=True)
                    if enabled(PATCH):
                        self.record()
                    result = None
                case _:
                    raise
        else:
            self.record()
        return result
";
    common::assert_matches_cpython_marshal(source, "match_handler_raise_join_control.py");
}
