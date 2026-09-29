events = []
a = True
b = False
c = True
if b:
    events.append('first')
elif a or c:
    assert a
