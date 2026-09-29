def target(self, ready, selected, enabled):
    if ready:
        if selected:
            value = build()
            if enabled:
                self.value = value
        else:
            try:
                self.value = build()
            except Exception:
                recover()
    return self.value
