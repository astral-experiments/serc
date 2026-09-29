def empty_read(self, manager):
    with manager as stream:
        while 1:
            item = stream.read()
            if not item: break
    self.finish()


def empty_lines(self, manager):
    with manager as stream:
        while 1:
            items = stream.readlines()
            if items == []: break
    self.finish()
