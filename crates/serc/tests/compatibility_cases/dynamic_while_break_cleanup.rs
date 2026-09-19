use crate::common;

#[test]
fn matches_cpython_dynamic_while_two_breaks_following_call() {
    common::assert_matches_cpython_marshal(
        r"async def drain(pending, expired, work, finish):
    try:
        while pending():
            if expired():
                break
            done = await work()
            if not done:
                break
    except Exception:
        raise
    finish()
    return None
",
        "dynamic_while_two_breaks_following_call.py",
    );
}

#[test]
fn matches_cpython_attribute_while_two_breaks_following_collection() {
    common::assert_matches_cpython_marshal(
        r"class DrainState:
    async def drain(self, *, timeout_seconds):
        deadline = timeout_seconds
        self._collect_completed()
        try:
            while self._pending_leases:
                remaining = deadline - self._clock()
                if remaining <= 0:
                    break
                done = await self._wait(remaining)
                if not done:
                    break
        except Exception:
            raise
        self._collect_completed()
        return self._pending_error()
",
        "attribute_while_two_breaks_following_collection.py",
    );
}

#[test]
fn matches_cpython_attribute_while_two_breaks_without_except() {
    common::assert_matches_cpython_marshal(
        r"class DrainState:
    async def drain(self, *, timeout_seconds):
        deadline = timeout_seconds
        self._collect_completed()
        while self._pending_leases:
            remaining = deadline - self._clock()
            if remaining <= 0:
                break
            done = await self._wait(remaining)
            if not done:
                break
        self._collect_completed()
        return self._pending_error()
",
        "attribute_while_two_breaks_without_except.py",
    );
}
