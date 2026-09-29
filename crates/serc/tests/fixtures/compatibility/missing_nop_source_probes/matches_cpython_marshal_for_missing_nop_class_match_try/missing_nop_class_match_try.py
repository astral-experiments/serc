from __future__ import annotations

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
