def target(self, ready, selected, enabled):
    if ready:
        if selected:
            if enabled:
                return self.value
        else:
            self.value = build()
    return self.value
