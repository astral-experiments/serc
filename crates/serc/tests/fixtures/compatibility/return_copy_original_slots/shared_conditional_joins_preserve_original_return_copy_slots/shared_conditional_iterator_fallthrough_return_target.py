def target(self, ready, selected, values):
    if ready:
        if selected:
            self.value = build()
            if self.value is None:
                for value in values:
                    consume(value)
        else:
            self.value = build()
    return self.value
