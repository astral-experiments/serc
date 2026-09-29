def f(first, second):
 if first:
  assert first
 elif first or second:
  assert second
 else:
  assert first
 action()
