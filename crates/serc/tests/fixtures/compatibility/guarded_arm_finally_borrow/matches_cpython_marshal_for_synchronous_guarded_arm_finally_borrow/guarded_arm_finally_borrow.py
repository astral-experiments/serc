from __future__ import annotations

class Refresher:
    @classmethod
    def refresh(cls) -> None:
        source = cls.source
        if source is None:
            raise RuntimeError('missing source')
        interval = cls.interval_for(source)
        try:
            if source == REMOTE:
                payload, interval = cls.fetch()
                if payload:
                    cls.value = cls.decode(payload)
            else:
                cls.value = cls.load()
        except Exception:
            if source == REMOTE:
                cls.token = None
            log.warning('refresh failed', extra={'source': source}, exc_info=True)
        finally:
            cls.next_refresh = cls.clock() + interval
