use crate::common;

#[test]
fn matches_cpython_marshal_for_missing_nop_class_match_try() {
    let source = r"from __future__ import annotations

def build_capture(First, Second, lookup, record, original):
    def capture(value, *, mode=None, token=None):
        try:
            match value:
                case First() as first:
                    extra = lookup(first.key)
                    record(first.to_dict(), extra=extra)
                case Second() as second:
                    record(second.to_dict())
        except Exception:
            pass
        return original(value, mode=mode, token=token)
    return capture
";
    common::assert_matches_cpython_marshal(source, "missing_nop_class_match_try.py");
}

#[test]
fn matches_cpython_marshal_for_missing_nop_try_assert_alternative() {
    let source = r"def choose(self):
    try:
        if not self.preferred.empty():
            item = self.preferred.get_nowait()
            assert item.metadata.private is not None
        else:
            item = self.fallback.get_nowait()
    except EmptyQueue:
        return None
    return item
";
    common::assert_matches_cpython_marshal(source, "missing_nop_try_assert_alternative.py");
}

#[test]
fn matches_cpython_marshal_for_missing_nop_loop_dispatch() {
    let source = r"from __future__ import annotations

import hashlib

def fingerprint(chunks):
    hasher = hashlib.sha256()
    if hasattr(chunks, 'read'):
        source = chunks
        while True:
            chunk = source.read(4096)
            if not chunk:
                break
            hasher.update(chunk)
    else:
        for chunk in chunks:
            hasher.update(chunk)
    return hasher.hexdigest()
";
    common::assert_matches_cpython_marshal(source, "missing_nop_loop_dispatch.py");
}
